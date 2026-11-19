// ABOUTME: A group owner with coach access becomes the coach of their own coachless group (carnet#1650)
// ABOUTME: The attach a coach-access grant makes, plus the same room line; no new notify event

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Becoming the coach of a coachless group.
//!
//! A group made before its owner held coach access has no human coach, and
//! a grant attaches the owner to such groups only at the moment it lands. An
//! owner who holds coach access *now* and finds their group still coachless
//! claims the seat here, through the same attach a grant makes
//! ([`crate::coach_access`]) and with the same line in the room, so the
//! athletes read that their coach is now here.

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_groups::creation_policy::creator_coaches_group;
use std::slice;
use tracing::info;
use uuid::Uuid;

use crate::coach_access::{attach_coach, CoachAccessNotices};

/// Make `user_id` the human coach of `group_id` in `tenant`.
///
/// The caller must own the group and hold coach access in its tenant (the
/// rule a group's creator is attached under), and the group must be active
/// with no coach. Once attached, `notices` writes the room line into every
/// copy of the group's chat (best-effort, logged); the coach is not mailed
/// or notified, having asked for it themselves.
///
/// # Errors
///
/// Returns not-found for an unknown group, one in another tenant (the group
/// lookup alone is cross-tenant, so the tenant is checked here) or an
/// archived one, a permission error for a caller who does not own the group
/// or lacks coach access, and a conflict when the group already has a coach.
pub async fn claim_group_coach(
    repos: &RepositoryRegistry,
    notices: &CoachAccessNotices,
    user_id: Uuid,
    group_id: &str,
    tenant: TenantId,
) -> AppResult<CoachingGroup> {
    let group = repos
        .groups
        .get_group(group_id, tenant)
        .await?
        .filter(|group| group.is_active && group.tenant_id == tenant.to_string())
        .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;
    let Some(user) = repos.users.get_global(user_id).await? else {
        return Err(AppError::auth_invalid(
            "The session's account no longer exists",
        ));
    };
    if group.owner_id != user_id || !creator_coaches_group(&user, tenant) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only the group's owner, with coach access, can become its coach",
        ));
    }
    if group.coach_user_id.is_some() {
        return Err(AppError::already_exists("A coach for this group"));
    }
    let attached = attach_coach(repos, group, user_id, tenant)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;
    let room_lines = notices
        .post_room_lines(repos, &user, slice::from_ref(&attached))
        .await;
    info!(
        user_id = %user_id,
        group_id = %attached.id,
        room_lines,
        "A group owner became the coach of their coachless group"
    );
    Ok(attached)
}
