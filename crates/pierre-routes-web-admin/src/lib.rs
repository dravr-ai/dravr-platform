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
//! - User lifecycle (`pending-users`, `users`, `approve-user`, `suspend-user`,
//!   `users/{id}/reset-password`, `promote`, `demote`, `admins`)
//! - Admin token CRUD (`tokens`, `tokens/{id}`, `tokens/{id}/revoke`,
//!   `tokens/{id}/rotate`)
//! - Per-user diagnostics (`users/{id}/rate-limit`, `activity`, `admin-profile`)
//! - Settings (`settings/auto-approval`)
//! - Tool selection (`tools/catalog`, `tools/tenant/{id}/*`, `tools/global-disabled`)
//! - Analytics (`analytics/recent-activity`)
//! - Billing / usage (`users/{id}/usage`, `cost-timeseries`,
//!   `tenants/{id}/usage|invoice`, `billing/export`)
//!
//! Everything is wired through [`WebAdminContext`] — a concrete state struct
//! collecting the Arc handles every handler pulls from the composition root's
//! `ServerContext`. This matches the precedent set by `AuthRoutesContext`,
//! `AdminApiContext`, and `ChatPipelineContext`.

#![warn(missing_docs)]

mod pre_approved_emails;
mod settings;

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
use pierre_config::environment::ServerConfig;
use pierre_config::security::llm_base_url_allowlist as config_llm_base_url_allowlist;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{TenantId, UserTier};
use pierre_database::RepositoryRegistry;
use pierre_llm::pricing::cost_for_aggregate;
use pierre_middleware::tenant_path::TenantPath;
use pierre_middleware::{extract_auth_from_headers, require_admin, McpAuthMiddleware};
use pierre_runtime_context::DataContext;
use pierre_services::admin_ops;
use pierre_services::user_approval::UserApprovalNotifier;
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
    /// Server config — `app_behavior` is read by the auto-approval handler.
    pub config: Arc<ServerConfig>,
    /// Data context bundle handed to every `admin_ops::*` service call.
    pub data: DataContext,
    /// Admin-token JWT signing secret — used by the create-admin-token path.
    pub admin_jwt_secret: Arc<str>,
    /// Tool-selection service — backs the `/api/admin/tools/*` surface.
    pub tool_selection: Arc<ToolSelectionService>,
    /// Notifier that emails and messages a just-approved user across their
    /// linked channels (injected by the composition root; `None` until wired).
    pub approval_notifier: Option<Arc<dyn UserApprovalNotifier>>,
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

/// Response for pending users list
#[derive(Serialize)]
struct PendingUsersResponse {
    count: usize,
    users: Vec<UserSummary>,
}

/// Response for all users list
#[derive(Serialize)]
struct AllUsersResponse {
    users: Vec<UserSummaryFull>,
    total_count: usize,
}

/// Response for admin tokens list
#[derive(Serialize)]
struct AdminTokensResponse {
    admin_tokens: Vec<AdminTokenSummary>,
    total_count: usize,
}

/// Admin token summary for listing.
///
/// Mirrors the `AdminToken` interface in `packages/shared-types/src/admin.ts`,
/// which the web console consumes. `permissions` and `usage_count` are declared
/// non-optional there, so omitting them here did not fail type-checking on
/// either side — it failed at runtime, when `ApiKeyDetails` dereferenced
/// `usage_count.toLocaleString()` on `undefined` and took the whole SPA down
/// through the root `ErrorBoundary`. `permissions` is emitted as a flat array via
/// `to_vec()` rather than the wrapper struct's `{ permissions: [...] }` shape,
/// because that array is what the client's `permissions.map()` expects.
#[derive(Serialize)]
struct AdminTokenSummary {
    id: String,
    service_name: String,
    service_description: Option<String>,
    permissions: Vec<AdminPermission>,
    is_active: bool,
    is_super_admin: bool,
    created_at: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    usage_count: u64,
    token_prefix: Option<String>,
}

/// Full user summary for listing all users
#[derive(Serialize)]
struct UserSummaryFull {
    id: String,
    email: String,
    display_name: Option<String>,
    tier: String,
    user_status: String,
    is_admin: bool,
    created_at: String,
    last_active: String,
    approved_at: Option<String>,
    approved_by: Option<String>,
}

/// User summary for listing
#[derive(Serialize)]
struct UserSummary {
    id: String,
    email: String,
    display_name: Option<String>,
    tier: String,
    created_at: String,
    last_active: String,
}

/// Request to approve a user
#[derive(Deserialize)]
struct ApproveUserRequest {
    reason: Option<String>,
}

/// Request to suspend a user
#[derive(Deserialize)]
struct SuspendUserRequest {
    reason: Option<String>,
}

/// Request to set a tool override
#[derive(Deserialize)]
struct SetToolOverrideRequest {
    tool_name: String,
    is_enabled: bool,
    reason: Option<String>,
}

/// Request to set a user's billing/quota tier
#[derive(Deserialize)]
struct SetUserTierRequest {
    /// `starter` | `professional` | `enterprise`
    tier: String,
}

/// Request to set a tenant's plan
#[derive(Deserialize)]
struct SetTenantPlanRequest {
    /// `starter` | `professional` | `enterprise`
    plan: String,
}

/// Request to create an admin token via web admin
#[derive(Deserialize)]
struct CreateAdminTokenWebRequest {
    service_name: String,
    service_description: Option<String>,
    permissions: Option<Vec<String>>,
    is_super_admin: Option<bool>,
    expires_in_days: Option<u64>,
}

/// Response for created admin token
#[derive(Serialize)]
struct CreateAdminTokenWebResponse {
    success: bool,
    token_id: String,
    service_name: String,
    jwt_token: String,
    token_prefix: String,
    is_super_admin: bool,
    expires_at: Option<String>,
}

/// Request body for rotating an admin token via web admin.
///
/// Every field is optional so a bare `POST` with an empty `{}` body — what the
/// console sends — rotates on the default one-year lifetime.
#[derive(Deserialize)]
struct RotateAdminTokenWebRequest {
    expires_in_days: Option<u64>,
}

/// Response for a rotated admin token.
///
/// The replacement token's fields sit at the top level, matching
/// [`CreateAdminTokenWebResponse`] rather than the nested `data.new_token`
/// envelope the service-token router returns: `ApiKeyDetails` reads
/// `data.jwt_token` straight off the mutation result to populate the
/// "copy your new token" modal.
#[derive(Serialize)]
struct RotateAdminTokenWebResponse {
    success: bool,
    message: String,
    old_token_id: String,
    token_id: String,
    service_name: String,
    jwt_token: String,
    token_prefix: String,
    is_super_admin: bool,
    expires_at: Option<String>,
}

/// Response for user status change operations
#[derive(Serialize)]
struct UserStatusChangeResponse {
    success: bool,
    message: String,
    user: UserStatusChangeUser,
}

/// User data in status change response
#[derive(Serialize)]
struct UserStatusChangeUser {
    id: String,
    email: String,
    user_status: String,
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

/// Query parameters for user activity endpoint
#[derive(Debug, Deserialize)]
pub struct UserActivityQuery {
    /// Number of days to look back (default: 30)
    pub days: Option<u32>,
}

/// Query parameters for the admin-token list endpoint.
#[derive(Debug, Deserialize)]
pub struct AdminTokensQuery {
    /// When true, revoked (inactive) tokens are returned alongside active ones.
    ///
    /// The console's token list requests this so it can render the Inactive
    /// badge and compute active/inactive counts client-side. The default of
    /// `false` keeps a bare `GET /api/admin/tokens` scoped to live tokens.
    #[serde(default)]
    pub include_inactive: bool,
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
            .route("/api/admin/pending-users", get(Self::handle_pending_users))
            .route("/api/admin/users", get(Self::handle_all_users))
            .route(
                "/api/admin/tokens",
                get(Self::handle_admin_tokens).post(Self::handle_create_admin_token),
            )
            .route(
                "/api/admin/tokens/{token_id}",
                get(Self::handle_get_admin_token),
            )
            .route(
                "/api/admin/tokens/{token_id}/revoke",
                post(Self::handle_revoke_admin_token),
            )
            .route(
                "/api/admin/tokens/{token_id}/rotate",
                post(Self::handle_rotate_admin_token),
            )
            .merge(pre_approved_emails::routes())
            .route(
                "/api/admin/approve-user/{user_id}",
                post(Self::handle_approve_user),
            )
            .route(
                "/api/admin/suspend-user/{user_id}",
                post(Self::handle_suspend_user),
            )
            .route(
                "/api/admin/users/{user_id}/reset-password",
                post(Self::handle_reset_user_password),
            )
            .route(
                "/api/admin/users/{user_id}/rate-limit",
                get(Self::handle_get_user_rate_limit),
            )
            // Rate-limit override + feature-flag admin routes live in
            // `pierre_routes_admin::AdminRoutes::cookie_admin_routes` and share
            // the same cookie-admin middleware mount as this router.
            .route(
                "/api/admin/users/{user_id}/activity",
                get(Self::handle_get_user_activity),
            )
            .route(
                "/api/admin/users/{user_id}/admin-profile",
                get(Self::handle_get_user_admin_profile),
            )
            .merge(settings::routes())
            // Tool selection routes (web admin versions with cookie auth)
            .route(
                "/api/admin/tools/catalog",
                get(Self::handle_get_tool_catalog),
            )
            .route(
                "/api/admin/tools/catalog/{tool_name}",
                get(Self::handle_get_tool_catalog_entry),
            )
            .route(
                "/api/admin/tools/global-disabled",
                get(Self::handle_get_global_disabled_tools),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}",
                get(Self::handle_get_tenant_tools),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/override",
                post(Self::handle_set_tool_override),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/override/{tool_name}",
                delete(Self::handle_remove_tool_override),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/summary",
                get(Self::handle_get_tool_summary),
            )
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
            // Per-user billing/quota tier (super-admin) + per-tenant plan
            .route(
                "/api/admin/users/{user_id}/tier",
                post(Self::handle_set_user_tier).delete(Self::handle_clear_user_tier),
            )
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
                "/api/admin/users/{user_id}/cost-timeseries",
                get(Self::handle_get_user_cost_timeseries),
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

    /// Handle pending users listing for web admin users
    async fn handle_pending_users(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        // Authenticate and verify admin status
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Web admin listing pending users"
        );

        // Admin user listing shows all users across tenants.
        // Any authenticated admin needs full visibility to manage users.
        // Per-tenant isolation applies to data operations, not admin views.

        // Fetch users with Pending status
        let users = resources
            .repos
            .users
            .get_by_status("pending", None)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch pending users: {e}")))?;

        // Convert to summaries
        let user_summaries: Vec<UserSummary> = users
            .iter()
            .map(|user| UserSummary {
                id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                tier: user.tier.to_string(),
                created_at: user.created_at.to_rfc3339(),
                last_active: user.last_active.to_rfc3339(),
            })
            .collect();

        let count = user_summaries.len();

        info!("Retrieved {count} pending users for web admin");

        Ok((
            StatusCode::OK,
            Json(PendingUsersResponse {
                count,
                users: user_summaries,
            }),
        )
            .into_response())
    }

    /// Handle listing all users for web admin users
    async fn handle_all_users(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        // Authenticate and verify admin status
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Web admin listing all users"
        );

        // Admin user listing shows all users across tenants.
        // Any authenticated admin needs full visibility to manage users.
        // Per-tenant isolation applies to data operations, not admin views.
        let mut all_users = Vec::new();

        for status in ["active", "pending", "suspended"] {
            let users = resources
                .repos
                .users
                .get_by_status(status, None)
                .await
                .map_err(|e| AppError::internal(format!("Failed to fetch {status} users: {e}")))?;
            all_users.extend(users);
        }

        let users = all_users;

        // Convert to full summaries
        let user_summaries: Vec<UserSummaryFull> = users
            .iter()
            .map(|user| UserSummaryFull {
                id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                tier: user.tier.to_string(),
                user_status: user.user_status.to_string(),
                is_admin: user.is_admin,
                created_at: user.created_at.to_rfc3339(),
                last_active: user.last_active.to_rfc3339(),
                approved_at: user.approved_at.map(|d| d.to_rfc3339()),
                approved_by: user.approved_by.map(|id| id.to_string()),
            })
            .collect();

        let total_count = user_summaries.len();

        info!("Retrieved {total_count} users for web admin");

        Ok((
            StatusCode::OK,
            Json(AllUsersResponse {
                users: user_summaries,
                total_count,
            }),
        )
            .into_response())
    }

    /// Handle listing admin tokens for web admin users
    async fn handle_admin_tokens(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Query(params): Query<AdminTokensQuery>,
    ) -> Result<Response, AppError> {
        // Authenticate and verify admin status
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // Admin tokens are credentials for the whole platform, including
        // super-admin service tokens. `authenticate_admin` only proves *an*
        // admin role, so without this a tenant-scoped admin could enumerate and
        // revoke a super-admin token — privilege escalation by deletion. The
        // programmatic twin (pierre-routes-admin) has always gated this; this
        // cookie-auth surface had drifted.
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        info!(
            user_id = %auth.user_id,
            include_inactive = params.include_inactive,
            "Web admin listing admin tokens"
        );

        let tokens = resources
            .repos
            .admin
            .list_tokens(params.include_inactive)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch admin tokens: {e}")))?;

        // Convert to summaries
        let token_summaries: Vec<AdminTokenSummary> = tokens
            .iter()
            .map(|token| AdminTokenSummary {
                id: token.id.clone(),
                service_name: token.service_name.clone(),
                service_description: token.service_description.clone(),
                permissions: token.permissions.to_vec(),
                is_active: token.is_active,
                is_super_admin: token.is_super_admin,
                created_at: token.created_at.to_rfc3339(),
                expires_at: token.expires_at.map(|d| d.to_rfc3339()),
                last_used_at: token.last_used_at.map(|d| d.to_rfc3339()),
                usage_count: token.usage_count,
                token_prefix: Some(token.token_prefix.clone()),
            })
            .collect();

        let total_count = token_summaries.len();

        info!("Retrieved {total_count} admin tokens for web admin");

        Ok((
            StatusCode::OK,
            Json(AdminTokensResponse {
                admin_tokens: token_summaries,
                total_count,
            }),
        )
            .into_response())
    }

    /// Handle approving a user via web admin (cookie auth)
    async fn handle_approve_user(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Json(request): Json<ApproveUserRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin approving user"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result = admin_ops::approve_user(
            &resources.data,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
            request.reason.as_deref(),
        )
        .await?;

        // Notify the user: approval email + a message on each linked channel.
        if let Some(notifier) = resources.approval_notifier.as_ref() {
            notifier
                .notify_user_approved(user_uuid, &result.email, None)
                .await;
        }

        Ok((
            StatusCode::OK,
            Json(UserStatusChangeResponse {
                success: true,
                message: "User approved successfully".to_owned(),
                user: UserStatusChangeUser {
                    id: result.user_id,
                    email: result.email,
                    user_status: result.user_status,
                },
            }),
        )
            .into_response())
    }

    /// Handle suspending a user via web admin (cookie auth)
    async fn handle_suspend_user(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Json(request): Json<SuspendUserRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin suspending user"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result = admin_ops::suspend_user(
            &resources.data,
            auth.user_id,
            user_uuid,
            request.reason.as_deref(),
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(UserStatusChangeResponse {
                success: true,
                message: "User suspended successfully".to_owned(),
                user: UserStatusChangeUser {
                    id: result.user_id,
                    email: result.email,
                    user_status: result.user_status,
                },
            }),
        )
            .into_response())
    }

    /// Build a `CreateAdminTokenRequest` from the web request payload
    fn build_admin_token_request(request: CreateAdminTokenWebRequest) -> CreateAdminTokenRequest {
        let permissions = request.permissions.map(|perms| {
            perms
                .iter()
                .filter_map(|p| p.parse::<AdminPermission>().ok())
                .collect::<Vec<_>>()
        });

        CreateAdminTokenRequest {
            service_name: request.service_name,
            service_description: request.service_description,
            permissions,
            expires_in_days: request.expires_in_days,
            is_super_admin: request.is_super_admin.unwrap_or(false),
            tenant_id: None,
        }
    }

    /// Handle creating an admin token via web admin (cookie auth)
    async fn handle_create_admin_token(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Json(request): Json<CreateAdminTokenWebRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        // Only super-admins can create super-admin tokens
        if request.is_super_admin.unwrap_or(false) {
            admin_ops::require_super_admin(auth.user_id, &resources.data).await?;
        }

        info!(
            user_id = %auth.user_id,
            service_name = %request.service_name,
            "Web admin creating admin token"
        );

        let token_request = Self::build_admin_token_request(request);

        // Generate token using database method
        let generated_token = resources
            .repos
            .admin
            .create_token(
                &token_request,
                &resources.admin_jwt_secret,
                &resources.jwks_manager,
            )
            .await
            .map_err(|e| AppError::internal(format!("Failed to create admin token: {e}")))?;

        info!(
            token_id = %generated_token.token_id,
            "Admin token created successfully via web admin"
        );

        Ok((
            StatusCode::CREATED,
            Json(CreateAdminTokenWebResponse {
                success: true,
                token_id: generated_token.token_id,
                service_name: generated_token.service_name,
                jwt_token: generated_token.jwt_token,
                token_prefix: generated_token.token_prefix,
                is_super_admin: generated_token.is_super_admin,
                expires_at: generated_token.expires_at.map(|t| t.to_rfc3339()),
            }),
        )
            .into_response())
    }

    /// Handle getting a specific admin token via web admin (cookie auth)
    async fn handle_get_admin_token(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(token_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // Same escalation risk as the list handler — see the note there.
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        info!(
            user_id = %auth.user_id,
            token_id = %token_id,
            "Web admin getting admin token details"
        );

        let token = resources
            .repos
            .admin
            .get_token_by_id(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch admin token: {e}")))?
            .ok_or_else(|| AppError::not_found(format!("Admin token {token_id}")))?;

        Ok((
            StatusCode::OK,
            Json(AdminTokenSummary {
                id: token.id,
                service_name: token.service_name,
                service_description: token.service_description,
                permissions: token.permissions.to_vec(),
                is_active: token.is_active,
                is_super_admin: token.is_super_admin,
                created_at: token.created_at.to_rfc3339(),
                expires_at: token.expires_at.map(|d| d.to_rfc3339()),
                last_used_at: token.last_used_at.map(|d| d.to_rfc3339()),
                usage_count: token.usage_count,
                token_prefix: Some(token.token_prefix),
            }),
        )
            .into_response())
    }

    /// Handle rotating an admin token via web admin (cookie auth)
    ///
    /// Deactivates the existing token and mints a replacement carrying the same
    /// service identity, super-admin flag, and tenant scope. The plaintext JWT
    /// is returned once, in this response, and is not recoverable afterwards.
    async fn handle_rotate_admin_token(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(token_id): Path<String>,
        request: Option<Json<RotateAdminTokenWebRequest>>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // Rotation both destroys the current credential and mints a new one at
        // the old token's privilege level — including super-admin. Same gate as
        // the list/get/revoke handlers; see the note on the list handler.
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        info!(
            user_id = %auth.user_id,
            token_id = %token_id,
            "Web admin rotating admin token"
        );

        let existing_token = resources
            .repos
            .admin
            .get_token_by_id(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch admin token: {e}")))?
            .ok_or_else(|| AppError::not_found(format!("Admin token {token_id}")))?;

        resources
            .repos
            .admin
            .deactivate_token(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to deactivate old token: {e}")))?;

        let expires_in_days = request
            .and_then(|Json(body)| body.expires_in_days)
            .unwrap_or(365);

        let token_request = CreateAdminTokenRequest {
            service_name: existing_token.service_name,
            service_description: existing_token.service_description,
            permissions: None,
            expires_in_days: Some(expires_in_days),
            is_super_admin: existing_token.is_super_admin,
            tenant_id: existing_token.tenant_id,
        };

        let new_token = resources
            .repos
            .admin
            .create_token(
                &token_request,
                &resources.admin_jwt_secret,
                &resources.jwks_manager,
            )
            .await
            .map_err(|e| AppError::internal(format!("Failed to generate new admin token: {e}")))?;

        info!(
            old_token_id = %token_id,
            new_token_id = %new_token.token_id,
            "Admin token rotated successfully via web admin"
        );

        Ok((
            StatusCode::OK,
            Json(RotateAdminTokenWebResponse {
                success: true,
                message: "Admin token rotated successfully".to_owned(),
                old_token_id: token_id,
                token_id: new_token.token_id,
                service_name: new_token.service_name,
                jwt_token: new_token.jwt_token,
                token_prefix: new_token.token_prefix,
                is_super_admin: new_token.is_super_admin,
                expires_at: new_token.expires_at.map(|t| t.to_rfc3339()),
            }),
        )
            .into_response())
    }

    /// Handle revoking an admin token via web admin (cookie auth)
    async fn handle_revoke_admin_token(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(token_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // The sharpest edge of the three: revoking a super-admin service token
        // is a denial-of-service against the platform's own operators, and it
        // was reachable by any admin-role account. See the list handler.
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        info!(
            user_id = %auth.user_id,
            token_id = %token_id,
            "Web admin revoking admin token"
        );

        // `deactivate_token` is an unconditional UPDATE that returns `()`, so a
        // miss is indistinguishable from a hit at the repository layer. Resolve
        // the token first so an unknown id answers 404 rather than reporting a
        // revocation that never touched a row.
        resources
            .repos
            .admin
            .get_token_by_id(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch admin token: {e}")))?
            .ok_or_else(|| AppError::not_found(format!("Admin token {token_id}")))?;

        resources
            .repos
            .admin
            .deactivate_token(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to revoke admin token: {e}")))?;

        info!(
            "Admin token {} revoked successfully via web admin",
            token_id
        );

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Admin token revoked successfully",
                "token_id": token_id
            })),
        )
            .into_response())
    }

    /// Handle password reset via web admin
    ///
    /// Issues a one-time reset token instead of returning a temporary password.
    /// The admin delivers the token to the user, who calls `POST /api/auth/complete-reset`
    /// with the token and their chosen new password. Token expires after 1 hour.
    async fn handle_reset_user_password(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin issuing password reset token"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result = admin_ops::generate_password_reset_token(
            &resources.data,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Password reset token issued",
                "data": {
                    "reset_token": result.reset_token,
                    "expires_in_seconds": result.expires_in_seconds,
                    "user_email": result.user_email,
                    "note": "Deliver this token to the user. They must call POST /api/auth/complete-reset with the token and their new password within 1 hour."
                }
            })),
        )
            .into_response())
    }

    /// Handle getting rate limit info for a user via web admin
    async fn handle_get_user_rate_limit(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let limits = admin_ops::compute_user_rate_limits(&resources.repos, user_uuid).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Rate limit information retrieved",
                "data": {
                    "user_id": limits.user_id,
                    "tier": limits.tier,
                    "rate_limits": {
                        "daily": {
                            "limit": limits.daily_limit,
                            "used": limits.daily_used,
                            "remaining": limits.daily_remaining,
                        },
                        "monthly": {
                            "limit": limits.monthly_limit,
                            "used": limits.monthly_used,
                            "remaining": limits.monthly_remaining,
                        },
                    },
                    "reset_times": {
                        "daily_reset": limits.daily_reset.to_rfc3339(),
                        "monthly_reset": limits.monthly_reset.to_rfc3339(),
                    },
                    "override_active": limits.override_active,
                    "override_note": limits.override_note,
                }
            })),
        )
            .into_response())
    }

    /// Handle GET /api/admin/users/{user_id}/admin-profile — returns the user's
    /// coaching persona, installed coaches, and joined groups for the admin
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

    /// Handle getting user activity via web admin
    async fn handle_get_user_activity(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Query(params): Query<UserActivityQuery>,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let activity =
            admin_ops::compute_user_activity(&resources.repos, user_uuid, params.days).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "User activity retrieved",
                "data": {
                    "user_id": activity.user_id,
                    "period_days": activity.period_days,
                    "total_requests": activity.total_requests,
                    "top_tools": activity.top_tools,
                }
            })),
        )
            .into_response())
    }

    // =========================================================================
    // Tool Selection Routes (web admin versions with cookie auth)
    // =========================================================================

    /// GET `/api/admin/tools/catalog` - List all tools in catalog
    async fn handle_get_tool_catalog(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let catalog = resources.tool_selection.get_catalog().await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved {} tools from catalog", catalog.len()),
                "data": catalog
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/catalog/{tool_name}` - Get single tool details
    async fn handle_get_tool_catalog_entry(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(tool_name): Path<String>,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let catalog = resources.tool_selection.get_catalog().await?;
        let entry = catalog
            .into_iter()
            .find(|e| e.tool_name == tool_name)
            .ok_or_else(|| AppError::not_found(format!("Tool '{tool_name}'")))?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved tool '{tool_name}'"),
                "data": entry
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/global-disabled` - List globally disabled tools
    async fn handle_get_global_disabled_tools(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let disabled_tools = resources.tool_selection.get_globally_disabled_tools();
        let count = disabled_tools.len();

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": if count == 0 {
                    "No tools are globally disabled".to_owned()
                } else {
                    format!("{count} tool(s) globally disabled via PIERRE_DISABLED_TOOLS")
                },
                "data": {
                    "disabled_tools": disabled_tools,
                    "count": count
                }
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/tenant/{tenant_id}` - Get effective tools for tenant
    async fn handle_get_tenant_tools(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        TenantPath(tenant_id): TenantPath,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant_id).await?;

        let tools = resources
            .tool_selection
            .get_effective_tools(tenant_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved {} effective tools for tenant {tenant_id}", tools.len()),
                "data": tools
            })),
        )
            .into_response())
    }

    /// POST `/api/admin/tools/tenant/{tenant_id}/override` - Set tool override
    async fn handle_set_tool_override(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        TenantPath(tenant_id): TenantPath,
        Json(request): Json<SetToolOverrideRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant_id).await?;

        info!(
            "Setting tool override: tenant={}, tool={}, enabled={}, by={}",
            tenant_id, request.tool_name, request.is_enabled, auth.user_id
        );

        let override_entry = resources
            .tool_selection
            .set_tool_override(
                tenant_id,
                &request.tool_name,
                request.is_enabled,
                auth.user_id,
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
                "message": format!("Tool '{}' {} for tenant {tenant_id}", request.tool_name, action),
                "data": override_entry
            })),
        )
            .into_response())
    }

    /// DELETE `/api/admin/tools/tenant/{tenant_id}/override/{tool_name}` - Remove override
    async fn handle_remove_tool_override(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        TenantPath(tenant_id): TenantPath,
        Path(tool_name): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant_id).await?;

        info!(
            "Removing tool override: tenant={}, tool={}, by={}",
            tenant_id, tool_name, auth.user_id
        );

        let deleted = resources
            .tool_selection
            .remove_tool_override(tenant_id, &tool_name)
            .await?;

        if deleted {
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "message": format!("Override removed for tool '{tool_name}' on tenant {tenant_id}")
                })),
            )
                .into_response())
        } else {
            Err(AppError::not_found(format!(
                "No override found for tool '{tool_name}' on tenant {tenant_id}"
            )))
        }
    }

    /// GET `/api/admin/tools/tenant/{tenant_id}/summary` - Get availability summary
    async fn handle_get_tool_summary(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        TenantPath(tenant_id): TenantPath,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant_id).await?;

        let summary = resources
            .tool_selection
            .get_availability_summary(tenant_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!(
                    "Tenant {tenant_id}: {}/{} tools enabled",
                    summary.enabled_tools, summary.total_tools
                ),
                "data": summary
            })),
        )
            .into_response())
    }

    /// POST `/api/admin/users/{user_id}/tier` - set a user's billing/quota tier.
    ///
    /// Super-admin only (billing-sensitive; mirrors the token surface). Writes
    /// `users.tier` and the anti-clobber override marker via the shared
    /// [`admin_ops::set_user_tier`].
    async fn handle_set_user_tier(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Json(request): Json<SetUserTierRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
        let tier = match request.tier.to_ascii_lowercase().as_str() {
            "starter" => UserTier::Starter,
            "professional" => UserTier::Professional,
            "enterprise" => UserTier::Enterprise,
            other => {
                return Err(AppError::invalid_input(format!(
                    "Unknown tier '{other}' — expected starter, professional, or enterprise"
                )));
            }
        };

        let note = format!("admin tier override via web console (by {})", auth.user_id);
        let updated = admin_ops::set_user_tier(
            &resources.repos,
            user_uuid,
            tier,
            Some(note),
            Some(auth.user_id),
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("User {} tier set to {}", updated.email, updated.tier),
                "data": {
                    "user_id": user_uuid.to_string(),
                    "email": updated.email,
                    "tier": updated.tier.as_str(),
                }
            })),
        )
            .into_response())
    }

    /// DELETE `/api/admin/users/{user_id}/tier` - clear the tier override so the
    /// billing webhook drives the tier again. Super-admin only.
    async fn handle_clear_user_tier(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
        let removed = admin_ops::clear_user_tier_override(&resources.repos, user_uuid).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": if removed {
                    "Tier override cleared; billing webhook will re-drive the tier".to_owned()
                } else {
                    "No tier override existed".to_owned()
                },
                "data": { "removed": removed }
            })),
        )
            .into_response())
    }

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

    /// `GET /api/admin/users/{user_id}/cost-timeseries?from=<rfc3339>`
    async fn handle_get_user_cost_timeseries(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Query(q): Query<UsageRangeQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        Self::authorize_admin_for_user(&resources, auth.user_id, &user_id).await?;
        let from = resolve_start(q.from.as_deref())?.to_rfc3339();
        let daily = resources
            .repos
            .llm_usage
            .get_llm_usage_daily_series_by_user(&user_id, &from)
            .await?;
        Ok((
            StatusCode::OK,
            Json(UserCostTimeseriesResponse {
                user_id,
                from,
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
