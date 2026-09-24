// ABOUTME: Who may update a coaching group's settings over REST — owners and admins every field
// ABOUTME: The group's attached human coach may change its weekly digest mode and nothing else
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use uuid::Uuid;

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{GroupDigestMode, UpdateGroupRequest};
use pierre_core::models::TenantId;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};

use crate::groups::GroupRoutes;

/// Refuse `request` unless `user_id` may make it on the group.
///
/// An owner or admin may change every setting. The human coach attached to
/// the group (`coaching_groups.coach_user_id`) is not a member of it, and may
/// change the weekly digest mode — [`GroupDigestMode::may_change`] is the one
/// rule for that — and no other field: a request that carries anything else
/// is refused whole, so the coach never renames, re-agents or archives a
/// group through this route. Anyone else is refused exactly as before the
/// coach could write: a plain member with `PermissionDenied`, a stranger with
/// "not found".
///
/// # Errors
///
/// Returns `PermissionDenied` or `NotFound` as described, or the repository
/// error when the membership or the group cannot be read.
pub async fn authorize_group_update<C: GroupsCtx + MiddlewareCtx>(
    resources: &Arc<C>,
    group_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    request: &UpdateGroupRequest,
) -> Result<(), AppError> {
    let membership = resources
        .repos()
        .groups
        .get_member(group_id, user_id)
        .await?;
    let role = membership.as_ref().map(|m| m.role);
    if role.is_some_and(|r| r.can_modify_settings()) {
        return Ok(());
    }

    let is_attached_coach = resources
        .group_service()
        .get_group(group_id, tenant_id)
        .await?
        .is_some_and(|group| group.coach_user_id == Some(user_id));
    if request.changes_only_digest_mode() && GroupDigestMode::may_change(role, is_attached_coach) {
        return Ok(());
    }
    if is_attached_coach {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "The group's coach may change only its weekly digest",
        ));
    }
    GroupRoutes::admin_membership(membership).map(drop)
}
