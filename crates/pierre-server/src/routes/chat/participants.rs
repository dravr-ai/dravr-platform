// ABOUTME: Handlers for /api/chat/conversations/{id}/participants — list, add, remove
// ABOUTME: Membership-gated: only a participant may see or change who is in the thread
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{ParticipantRole, TenantId};
use pierre_middleware::AuthenticatedUser;

use super::common::get_tenant_id;
use super::dto::{AddParticipantRequest, ParticipantListResponse, ParticipantResponse};

/// The membership check every participant route reuses: the caller must
/// already be in the conversation, in this tenant. A stranger gets the same
/// 404 the conversation routes give, so the existence of a thread is never
/// disclosed to someone outside it.
async fn require_participant(
    resources: &ServerContext,
    conversation_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Result<(), AppError> {
    resources
        .common
        .repos
        .chat
        .get_conversation(conversation_id, &user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))
        .map(|_| ())
}

/// List who is in the conversation, owner first.
pub async fn list_participants(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    require_participant(&resources, &conversation_id, auth.user_id, tenant_id).await?;

    let participants = resources
        .common
        .repos
        .chat
        .list_participants(&conversation_id, tenant_id)
        .await?
        .into_iter()
        .map(ParticipantResponse::from)
        .collect();

    Ok((
        StatusCode::OK,
        Json(ParticipantListResponse { participants }),
    )
        .into_response())
}

/// Add a user to the conversation.
///
/// Cross-tenant is refused explicitly: a conversation lives in one tenant
/// and every participant is a member of it, so a user with no `tenant_users`
/// row there is answered with 403 rather than silently written as a
/// membership the read predicate would never honour. Re-adding an existing
/// participant is idempotent and returns the row as it stands.
pub async fn add_participant(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Json(request): Json<AddParticipantRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    require_participant(&resources, &conversation_id, auth.user_id, tenant_id).await?;

    let target = Uuid::parse_str(request.user_id.trim())
        .map_err(|_| AppError::invalid_input("user_id must be a UUID"))?;

    let in_tenant = resources
        .common
        .repos
        .tenants
        .get_user_role(target, tenant_id)
        .await?
        .is_some();
    if !in_tenant {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Cannot add a user who is not a member of this conversation's tenant",
        ));
    }

    let participant = resources
        .common
        .repos
        .chat
        .add_participant(
            &conversation_id,
            tenant_id,
            &target.to_string(),
            &auth.user_id.to_string(),
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(ParticipantResponse::from(participant)),
    )
        .into_response())
}

/// Remove a member from the conversation.
///
/// The owner cannot be removed — 400, since the request is malformed rather
/// than forbidden: the thread has no meaning without them. A user who is
/// not a member of the thread is a 404.
pub async fn remove_participant(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, user_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    require_participant(&resources, &conversation_id, auth.user_id, tenant_id).await?;

    let target = Uuid::parse_str(user_id.trim())
        .map_err(|_| AppError::invalid_input("user_id must be a UUID"))?;

    let is_owner = resources
        .common
        .repos
        .chat
        .list_participants(&conversation_id, tenant_id)
        .await?
        .iter()
        .any(|p| p.role == ParticipantRole::Owner && p.user_id == target.to_string());
    if is_owner {
        return Err(AppError::invalid_input(
            "The conversation's owner cannot be removed",
        ));
    }

    let removed = resources
        .common
        .repos
        .chat
        .remove_participant(&conversation_id, tenant_id, &target.to_string())
        .await?;
    if !removed {
        return Err(AppError::not_found("Participant not found"));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}
