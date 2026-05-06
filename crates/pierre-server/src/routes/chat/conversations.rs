// ABOUTME: CRUD handlers for /api/chat/conversations — create/list/get/update/delete + messages
// ABOUTME: No LLM dispatch; pure chat_orchestration + chat repo calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::services::chat_orchestration;
use pierre_core::errors::ErrorCode;

use super::common::{authenticate, get_tenant_id};
use super::dto::{
    ConversationListResponse, ConversationResponse, ConversationSummaryResponse,
    CreateConversationRequest, ListConversationsQuery, MessageResponse, MessagesListResponse,
    UpdateConversationRequest,
};

/// Reject conversation creation when the caller is not an active member of
/// the requested group. `group_id` must be a real `coaching_group` the user
/// belongs to — otherwise the LLM would be handed peer fitness data the
/// caller has no relationship to.
async fn verify_group_membership(
    resources: &Arc<ServerContext>,
    group_id: &str,
    user_id: Uuid,
) -> Result<(), AppError> {
    let member = resources.repos.groups.get_member(group_id, user_id).await?;
    match member {
        Some(m) if m.left_at.is_none() => Ok(()),
        _ => Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Cannot attach conversation to a group you don't belong to",
        )),
    }
}

/// Create a new conversation.
pub async fn create_conversation(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;
    let user_id_str = auth.user_id.to_string();

    // Enforce max_active_conversations limit from admin config
    if let Some(ref admin_config) = resources.admin_config {
        let max_conversations = admin_config
            .get_value(
                "usage_quotas.max_active_conversations",
                Some(&tenant_id.to_string()),
            )
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .unwrap_or(2);

        let current_count = resources
            .repos
            .chat
            .count_conversations(&user_id_str, tenant_id)
            .await?;
        if current_count >= max_conversations {
            return Err(AppError::quota_exceeded(
                "max_active_conversations",
                current_count,
                max_conversations,
                "",
            ));
        }
    }

    // Verify group membership when caller asks to attach a group_id —
    // a user can only create a conversation scoped to a group they belong to.
    if let Some(gid) = request.group_id.as_deref() {
        verify_group_membership(&resources, gid, auth.user_id).await?;
    }

    let result = chat_orchestration::create_conversation(
        resources.repos.chat.as_ref(),
        &user_id_str,
        tenant_id,
        &request.title,
        request.model.as_deref(),
        request.coach_id.as_deref(),
        request.group_id.as_deref(),
    )
    .await?;

    let conv = result.conversation;
    let response = ConversationResponse {
        id: conv.id,
        title: conv.title,
        model: conv.model,
        coach_id: conv.coach_id,
        group_id: conv.group_id,
        total_tokens: conv.total_tokens,
        created_at: conv.created_at,
        updated_at: conv.updated_at,
    };

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// List the caller's conversations, paginated via query params.
pub async fn list_conversations(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Query(query): Query<ListConversationsQuery>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;

    let conversations = resources
        .repos
        .chat
        .list_conversations(
            &auth.user_id.to_string(),
            tenant_id,
            query.limit,
            query.offset,
        )
        .await?;

    let total = conversations.len();
    let response = ConversationListResponse {
        conversations: conversations
            .into_iter()
            .map(|c| ConversationSummaryResponse {
                id: c.id,
                title: c.title,
                model: c.model,
                message_count: c.message_count,
                total_tokens: c.total_tokens,
                created_at: c.created_at,
                updated_at: c.updated_at,
            })
            .collect(),
        total,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Fetch a single conversation by id (scoped to the caller's tenant).
pub async fn get_conversation(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;

    let conv = resources
        .repos
        .chat
        .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let response = ConversationResponse {
        id: conv.id,
        title: conv.title,
        model: conv.model,
        coach_id: conv.coach_id,
        group_id: conv.group_id,
        total_tokens: conv.total_tokens,
        created_at: conv.created_at,
        updated_at: conv.updated_at,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Rename a conversation, returning the updated record.
pub async fn update_conversation(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(request): Json<UpdateConversationRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;

    let updated = resources
        .repos
        .chat
        .update_conversation_title(
            &conversation_id,
            &auth.user_id.to_string(),
            tenant_id,
            &request.title,
        )
        .await?;

    if !updated {
        return Err(AppError::not_found("Conversation not found"));
    }

    // Fetch and return the updated conversation (proper REST response)
    let conv = resources
        .repos
        .chat
        .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("Conversation not found after update"))?;

    let response = ConversationResponse {
        id: conv.id,
        title: conv.title,
        model: conv.model,
        coach_id: conv.coach_id,
        group_id: conv.group_id,
        total_tokens: conv.total_tokens,
        created_at: conv.created_at,
        updated_at: conv.updated_at,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Hard-delete a conversation the caller owns.
pub async fn delete_conversation(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;

    let deleted = resources
        .repos
        .chat
        .delete_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?;

    if !deleted {
        return Err(AppError::not_found("Conversation not found"));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// List all messages in a conversation after verifying ownership.
pub async fn get_messages(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;

    // Verify user owns this conversation
    resources
        .repos
        .chat
        .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let messages = resources
        .repos
        .chat
        .get_messages(&conversation_id, &auth.user_id.to_string())
        .await?;

    let messages_list: Vec<MessageResponse> = messages
        .into_iter()
        .map(|m| MessageResponse {
            id: m.id,
            role: m.role,
            content: m.content,
            token_count: m.token_count,
            created_at: m.created_at,
        })
        .collect();

    let response = MessagesListResponse {
        messages: messages_list,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
