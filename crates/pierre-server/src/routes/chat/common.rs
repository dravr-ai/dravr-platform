// ABOUTME: Shared tenant resolution helper for chat route handlers
// ABOUTME: Resolves the caller's tenant id, defaulting to user id when no tenant row exists
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::models::TenantId;

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
