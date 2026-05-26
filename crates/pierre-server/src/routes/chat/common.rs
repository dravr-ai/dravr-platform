// ABOUTME: Thin wrapper over the canonical pierre_runtime_context::resolve_tenant helper
// ABOUTME: Chat routes use Required — no user-id fallback; errors if user has no tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};

use crate::mcp::resources::ServerContext;

/// Resolve the caller's tenant id via the canonical resolver. Returns an
/// auth error if the user has no tenants — never fabricates an id from
/// the user uuid.
pub async fn get_tenant_id(
    auth: &AuthResult,
    resources: &Arc<ServerContext>,
) -> Result<TenantId, AppError> {
    require(resolve_tenant(resources, auth, TenantMode::Required).await?)
}
