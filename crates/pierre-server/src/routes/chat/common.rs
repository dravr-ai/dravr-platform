// ABOUTME: Thin wrapper over the canonical pierre_runtime_context::resolve_tenant helper
// ABOUTME: Chat routes use Required — no user-id fallback; errors if user has no tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};
use uuid::Uuid;

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

/// Reject group-scoped access when the caller is not an active member of
/// the group. `group_id` must be a real `coaching_group` the user belongs
/// to — otherwise the caller would be handed peer content and fitness data
/// they have no relationship to. Gates both conversation attachment and the
/// room-transcript read.
pub async fn verify_group_membership(
    resources: &Arc<ServerContext>,
    group_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Result<(), AppError> {
    let member = resources
        .common
        .repos
        .groups
        .get_member(group_id, user_id)
        .await?;
    if matches!(&member, Some(m) if m.left_at.is_none()) {
        return Ok(());
    }

    // The group's human coach can reach the group even though they are
    // not a member — they oversee the group through its coach persona, with
    // each member's data still gated by their own peer_sharing_consent.
    if let Some(group) = resources
        .common
        .repos
        .groups
        .get_group(group_id, tenant_id)
        .await?
    {
        if group.coach_user_id == Some(user_id) {
            return Ok(());
        }
    }

    Err(AppError::new(
        ErrorCode::PermissionDenied,
        "Cannot attach conversation to a group you don't belong to",
    ))
}
