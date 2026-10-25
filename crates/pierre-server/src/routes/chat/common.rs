// ABOUTME: Thin wrapper over the canonical pierre_runtime_context::resolve_tenant helper, plus the shared conversation wire shape
// ABOUTME: Chat routes use Required — no user-id fallback; errors if user has no tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};
use pierre_services::athlete_display_name::fetch_user_display_name;
use uuid::Uuid;

use super::dto::ConversationResponse;
use crate::mcp::resources::ServerContext;

/// The wire shape of a conversation that may be a coach's thread bound to one
/// athlete: that athlete's roster name is resolved alongside, so the three
/// `subject_*` fields go out together. One user read for such a thread, none
/// for any other.
pub(super) async fn conversation_response(
    resources: &ServerContext,
    conv: ConversationRecord,
) -> ConversationResponse {
    let subject_display_name = match conv.subject_user_id {
        Some(athlete) => Some(fetch_user_display_name(&resources.common.repos, athlete).await),
        None => None,
    };
    ConversationResponse::new(conv, subject_display_name)
}

/// Resolve the caller's tenant id via the canonical resolver. Returns an
/// auth error if the user has no tenants — never fabricates an id from
/// the user uuid.
///
/// # Errors
///
/// Returns the resolver's error, and an auth error for a user with no
/// tenant.
pub(crate) async fn get_tenant_id(
    auth: &AuthResult,
    resources: &Arc<ServerContext>,
) -> Result<TenantId, AppError> {
    require(resolve_tenant(resources, auth, TenantMode::Required).await?)
}

/// Reject group-scoped access unless the group admits the caller: a live
/// member of an active group, or its human coach
/// ([`CoachingGroupRepository::admits_member_or_coach`](pierre_database::repositories::CoachingGroupRepository::admits_member_or_coach)).
/// Otherwise the caller would be handed peer content and fitness data they
/// have no relationship to. The coach oversees the group through its agent
/// persona, with each member's data still gated by their own
/// `coach_sharing_consent` (a fellow member's read by `peer_sharing_consent`).
/// Gates both conversation attachment and the
/// room-transcript read.
pub(super) async fn verify_group_membership(
    resources: &Arc<ServerContext>,
    group_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Result<(), AppError> {
    if resources
        .common
        .repos
        .groups
        .admits_member_or_coach(group_id, user_id, tenant_id)
        .await?
    {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::PermissionDenied,
        "Cannot attach conversation to a group you don't belong to",
    ))
}
