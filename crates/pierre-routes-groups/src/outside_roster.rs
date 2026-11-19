// ABOUTME: The group roster an outside client acting for the group's coach is served over REST
// ABOUTME: A member who stopped sharing with the coach is left out; Dravr's own apps list every member
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::AppError;
use pierre_core::models::groups::GroupMember;
use pierre_core::models::TenantId;
use pierre_providers::ai_scope;
use pierre_runtime_context::GroupsCtx;

/// `members` as the group's roster is served to `auth`: an outside client
/// (an API key, an MCP token, an OAuth application, a request declared
/// outside Dravr's own surfaces) acting for the group's coach is never told
/// of a member who stopped sharing with the coach (EFVP 2026-11-17,
/// decision 9.4c). Anyone else is served every member.
///
/// # Errors
///
/// The repository error when the group cannot be read.
pub(crate) async fn members_served_to<C: GroupsCtx>(
    resources: &Arc<C>,
    auth: &AuthResult,
    group_id: &str,
    tenant_id: TenantId,
    mut members: Vec<GroupMember>,
) -> Result<Vec<GroupMember>, AppError> {
    if !ai_scope::served_outside(auth.acts_for_outside_client()) {
        return Ok(members);
    }
    if let Some(group) = resources
        .group_service()
        .get_group(group_id, tenant_id)
        .await?
    {
        let unnamed = group.unnamed_to_outside_client_of(auth.user_id, &members);
        members.retain(|member| !unnamed.contains(&member.user_id));
    }
    Ok(members)
}
