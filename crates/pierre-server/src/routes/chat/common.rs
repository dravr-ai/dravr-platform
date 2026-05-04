// ABOUTME: Shared auth + tenant resolution helpers for chat route handlers
// ABOUTME: Keeps extract_auth_from_headers plumbing in one place so each sibling module stays thin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::http::HeaderMap;
use pierre_auth::auth::AuthResult;
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::middleware::extract_auth_from_headers;
use crate::models::TenantId;

/// Extract and authenticate the user from Authorization header or cookie.
///
/// Thin wrapper over [`extract_auth_from_headers`] so every chat handler
/// calls the same helper.
pub async fn authenticate(
    headers: &HeaderMap,
    resources: &Arc<ServerContext>,
) -> Result<AuthResult, AppError> {
    extract_auth_from_headers(headers, resources).await
}

/// Resolve the caller's tenant id, defaulting to their user id when no
/// tenant row is attached to the account.
pub async fn get_tenant_id(
    user_id: Uuid,
    resources: &Arc<ServerContext>,
) -> Result<TenantId, AppError> {
    let tenants = resources.repos.tenants.list_for_user(user_id).await?;
    Ok(tenants
        .first()
        .map_or_else(|| TenantId::from(user_id), |t| t.id))
}
