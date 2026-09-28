// ABOUTME: Admin API routes for per-tenant MCP tool selection management
// ABOUTME: Enables admins to view, configure, and override tool availability per tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool selection admin routes for managing MCP tool availability per tenant.
//!
//! This module provides REST endpoints for:
//! - Viewing the tool catalog
//! - Managing per-tenant tool overrides
//! - Checking globally disabled tools
//!
//! One handler per operation: [`ToolSelectionRoutes::routes`] mounts them
//! under `/admin/tools` behind admin-token auth, and
//! [`ToolSelectionRoutes::console_routes`] mounts the tenant and global ones
//! under `/api/admin/tools` behind the console's session auth.
//!
//! A tenant's tools are managed by that tenant's admins: a super-admin reaches
//! every tenant, any other caller only a tenant its token is bound to or its
//! operator belongs to — which is how a console session of a plain Admin
//! reaches the tenants it administers.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_middleware::tenant_path::TenantPath;
use pierre_runtime_context::MiddlewareCtx;
use serde::{Deserialize, Serialize};
use tokio::task::yield_now;
use tracing::info;

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_tool_runtime::tool_selection::ToolSelectionService;

/// Require the caller to administer `tenant_id`.
///
/// A super-admin reaches every tenant. Any other caller reaches a tenant its
/// token is bound to, or one its operator — the signed-in admin of a console
/// session — is a member of. A service token bound to no tenant and acting for
/// no one reaches none.
async fn require_tenant_tool_access(
    repos: &RepositoryRegistry,
    admin_token: &ValidatedAdminToken,
    tenant_id: TenantId,
) -> AppResult<()> {
    if admin_token.is_super_admin
        || admin_token.tenant_id.as_deref() == Some(tenant_id.to_string().as_str())
    {
        return Ok(());
    }
    if let Some(operator) = admin_token.operator_user_id {
        let tenants = repos.tenants.list_for_user(operator).await?;
        if tenants.iter().any(|tenant| tenant.id == tenant_id) {
            return Ok(());
        }
    }
    Err(AppError::new(
        ErrorCode::PermissionDenied,
        "Tool settings of this tenant are managed by its own admins",
    ))
}

/// Context for tool selection routes
#[derive(Clone)]
pub struct ToolSelectionContext {
    /// Tool selection service for business logic
    pub tool_selection: Arc<ToolSelectionService>,
    /// Repository registry, read to check the caller's tenant membership
    pub repos: Arc<RepositoryRegistry>,
}

/// Response wrapper for tool selection endpoints
#[derive(Debug, Serialize)]
pub struct ToolSelectionResponse<T> {
    /// Whether the operation succeeded
    pub success: bool,
    /// Human-readable message
    pub message: String,
    /// Response data (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T: Serialize> ToolSelectionResponse<T> {
    fn success(message: impl Into<String>, data: T) -> Self {
        Self {
            success: true,
            message: message.into(),
            data: Some(data),
        }
    }
}

/// Response for globally disabled tools
#[derive(Debug, Serialize)]
pub struct GlobalDisabledToolsResponse {
    /// List of tool names disabled via `PIERRE_DISABLED_TOOLS`
    pub disabled_tools: Vec<String>,
    /// Number of disabled tools
    pub count: usize,
}

/// Tool selection admin routes
pub struct ToolSelectionRoutes;

impl ToolSelectionRoutes {
    /// Create all tool selection routes
    pub fn routes(context: ToolSelectionContext) -> Router {
        let context = Arc::new(context);

        Router::new()
            // Catalog routes (read-only)
            .route("/admin/tools/catalog", get(Self::handle_get_catalog))
            .route(
                "/admin/tools/catalog/{tool_name}",
                get(Self::handle_get_catalog_entry),
            )
            // Tenant configuration routes
            .route(
                "/admin/tools/tenant/{tenant_id}",
                get(Self::handle_get_tenant_tools),
            )
            .route(
                "/admin/tools/tenant/{tenant_id}/override",
                post(Self::handle_set_override),
            )
            .route(
                "/admin/tools/tenant/{tenant_id}/override/{tool_name}",
                delete(Self::handle_remove_override),
            )
            .route(
                "/admin/tools/tenant/{tenant_id}/summary",
                get(Self::handle_get_summary),
            )
            // Global status
            .route(
                "/admin/tools/global-disabled",
                get(Self::handle_get_global_disabled),
            )
            .with_state(context)
    }

    /// The console's tool-availability tab behind session auth: the same
    /// handlers as [`Self::routes`] for the tenant and global operations.
    ///
    /// The session middleware is generic over [`MiddlewareCtx`]; the
    /// composition root passes `Arc<ServerContext>` as its state.
    pub fn console_routes<C>(context: ToolSelectionContext, resources: &Arc<C>) -> Router
    where
        C: MiddlewareCtx,
    {
        let context = Arc::new(context);
        Router::new()
            .route(
                "/api/admin/tools/tenant/{tenant_id}",
                get(Self::handle_get_tenant_tools),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/override",
                post(Self::handle_set_override),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/override/{tool_name}",
                delete(Self::handle_remove_override),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/summary",
                get(Self::handle_get_summary),
            )
            .route(
                "/api/admin/tools/global-disabled",
                get(Self::handle_get_global_disabled),
            )
            .with_state(context)
            .layer(middleware::from_fn_with_state(
                Arc::clone(resources),
                pierre_middleware::cookie_admin_middleware::<C>,
            ))
    }

    /// GET /admin/tools/catalog - List all tools in catalog
    async fn handle_get_catalog(
        State(context): State<Arc<ToolSelectionContext>>,
        Extension(admin_token): Extension<ValidatedAdminToken>,
    ) -> AppResult<impl IntoResponse> {
        admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

        let catalog = context.tool_selection.get_catalog().await?;

        Ok((
            StatusCode::OK,
            Json(ToolSelectionResponse::success(
                format!("Retrieved {} tools from catalog", catalog.len()),
                catalog,
            )),
        ))
    }

    /// GET `/admin/tools/catalog/{tool_name}` - Get single tool details
    async fn handle_get_catalog_entry(
        State(context): State<Arc<ToolSelectionContext>>,
        Extension(admin_token): Extension<ValidatedAdminToken>,
        Path(tool_name): Path<String>,
    ) -> AppResult<impl IntoResponse> {
        admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

        let catalog = context.tool_selection.get_catalog().await?;
        let entry = catalog
            .into_iter()
            .find(|e| e.tool_name == tool_name)
            .ok_or_else(|| AppError::not_found(format!("Tool '{tool_name}'")))?;

        Ok((
            StatusCode::OK,
            Json(ToolSelectionResponse::success(
                format!("Retrieved tool '{tool_name}'"),
                entry,
            )),
        ))
    }

    /// GET `/admin/tools/tenant/{tenant_id}` - Get effective tools for tenant
    async fn handle_get_tenant_tools(
        State(context): State<Arc<ToolSelectionContext>>,
        Extension(admin_token): Extension<ValidatedAdminToken>,
        TenantPath(tenant_id): TenantPath,
    ) -> AppResult<impl IntoResponse> {
        require_tenant_tool_access(&context.repos, &admin_token, tenant_id).await?;

        let tools = context
            .tool_selection
            .get_effective_tools(tenant_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(ToolSelectionResponse::success(
                format!(
                    "Retrieved {} effective tools for tenant {tenant_id}",
                    tools.len()
                ),
                tools,
            )),
        ))
    }

    /// POST `/admin/tools/tenant/{tenant_id}/override` - Set tool override
    async fn handle_set_override(
        State(context): State<Arc<ToolSelectionContext>>,
        Extension(admin_token): Extension<ValidatedAdminToken>,
        TenantPath(tenant_id): TenantPath,
        Json(request): Json<SetOverrideRequest>,
    ) -> AppResult<impl IntoResponse> {
        require_tenant_tool_access(&context.repos, &admin_token, tenant_id).await?;

        info!(
            "Setting tool override: tenant={}, tool={}, enabled={}, by={}",
            tenant_id, request.tool_name, request.is_enabled, admin_token.token_id
        );

        // An override is attributed to the person who set it: the signed-in
        // admin of a console session, the approving operator of a device login.
        let admin_user_id = admin_token.operator_user_id.ok_or_else(|| {
            AppError::invalid_input(
                "Tool overrides are attributed to an operator; this token acts for none",
            )
        })?;

        let override_entry = context
            .tool_selection
            .set_tool_override(
                tenant_id,
                &request.tool_name,
                request.is_enabled,
                admin_user_id,
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
            Json(ToolSelectionResponse::success(
                format!(
                    "Tool '{}' {} for tenant {tenant_id}",
                    request.tool_name, action
                ),
                override_entry,
            )),
        ))
    }

    /// DELETE `/admin/tools/tenant/{tenant_id}/override/{tool_name}` - Remove override
    async fn handle_remove_override(
        State(context): State<Arc<ToolSelectionContext>>,
        Extension(admin_token): Extension<ValidatedAdminToken>,
        Path((tenant_id, tool_name)): Path<(String, String)>,
    ) -> AppResult<impl IntoResponse> {
        // Two path segments, so `TenantPath` (one segment) cannot read the
        // tenant here; the id is parsed the same way it parses one.
        let tenant_id = TenantId::parse_str(&tenant_id)
            .map_err(|_| AppError::invalid_input("Invalid tenant ID in path"))?;
        require_tenant_tool_access(&context.repos, &admin_token, tenant_id).await?;

        info!(
            "Removing tool override: tenant={}, tool={}, by={}",
            tenant_id, tool_name, admin_token.token_id
        );

        let deleted = context
            .tool_selection
            .remove_tool_override(tenant_id, &tool_name)
            .await?;

        if deleted {
            Ok((
                StatusCode::OK,
                Json(ToolSelectionResponse::<()>::success(
                    format!("Override removed for tool '{tool_name}' on tenant {tenant_id}"),
                    (),
                )),
            ))
        } else {
            Err(AppError::not_found(format!(
                "No override found for tool '{tool_name}' on tenant {tenant_id}"
            )))
        }
    }

    /// GET `/admin/tools/tenant/{tenant_id}/summary` - Get availability summary
    async fn handle_get_summary(
        State(context): State<Arc<ToolSelectionContext>>,
        Extension(admin_token): Extension<ValidatedAdminToken>,
        TenantPath(tenant_id): TenantPath,
    ) -> AppResult<impl IntoResponse> {
        require_tenant_tool_access(&context.repos, &admin_token, tenant_id).await?;

        let summary = context
            .tool_selection
            .get_availability_summary(tenant_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(ToolSelectionResponse::success(
                format!(
                    "Tenant {tenant_id}: {}/{} tools enabled",
                    summary.enabled_tools, summary.total_tools
                ),
                summary,
            )),
        ))
    }

    /// GET `/admin/tools/global-disabled` - List `PIERRE_DISABLED_TOOLS` values
    ///
    /// Open to every admin: it is the process's own deployment switch, which
    /// every tenant's tool tab shows beside its overrides.
    async fn handle_get_global_disabled(
        State(context): State<Arc<ToolSelectionContext>>,
    ) -> AppResult<impl IntoResponse> {
        // Yield to satisfy async requirement (Axum handlers must be async)
        yield_now().await;

        let disabled_tools = context.tool_selection.get_globally_disabled_tools();
        let count = disabled_tools.len();

        Ok((
            StatusCode::OK,
            Json(ToolSelectionResponse::success(
                if count == 0 {
                    "No tools are globally disabled".to_owned()
                } else {
                    format!("{count} tool(s) globally disabled via PIERRE_DISABLED_TOOLS")
                },
                GlobalDisabledToolsResponse {
                    disabled_tools,
                    count,
                },
            )),
        ))
    }
}

/// Request body for setting a tool override
#[derive(Debug, Deserialize)]
pub struct SetOverrideRequest {
    /// Name of the tool to override
    pub tool_name: String,
    /// Whether the tool should be enabled
    pub is_enabled: bool,
    /// Optional reason for the override
    pub reason: Option<String>,
}
