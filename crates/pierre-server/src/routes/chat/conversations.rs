// ABOUTME: CRUD handlers for /api/chat/conversations — create/list/get/update/delete + messages
// ABOUTME: No LLM dispatch; pure chat_pipeline persistence + chat repo calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use pierre_chat_pipeline::stages::persistence::create_conversation as create_conversation_row;
use pierre_config::constants::usage_quotas::DEFAULT_MAX_ACTIVE_CONVERSATIONS;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup, ConfigLookupScope};
use pierre_services::coach_selection::{record_coach_selection, CoachSelectionSource};
use pierre_services::locale::resolve_user_locale;

use super::common::{get_tenant_id, verify_group_membership};
use super::dto::{preview_text, resolve_stored_blocks};
use super::dto::{
    ConversationListResponse, ConversationResponse, ConversationSummaryResponse,
    CreateConversationRequest, LastMessageResponse, ListConversationsQuery, MessageFeedbackEntry,
    MessageResponse, MessagesListResponse, UpdateConversationRequest,
};

/// Best-effort `coach_assignments.use_count++` for REST-created conversations,
/// via the shared selection recorder that also emits `coach.selected`.
/// Logs and swallows errors so a transient DB hiccup doesn't fail the user-
/// visible conversation create. `record_coach_selection` logs the
/// coach-not-visible case itself and emits nothing for it.
async fn record_coach_usage_best_effort(
    resources: &ServerContext,
    coach_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    if let Err(e) = record_coach_selection(
        resources.coaches_manager(),
        coach_id,
        user_id,
        tenant_id,
        CoachSelectionSource::ChatConversation,
    )
    .await
    {
        tracing::warn!(coach_id, error = %e, "failed to record coach usage");
    }
}

/// Create a new conversation.
pub async fn create_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();

    // Enforce max_active_conversations. Degrade to the registered
    // default when admin config is unavailable rather than skipping the
    // limit entirely.
    let admin_config: &dyn AdminConfigLookup = match resources.coach.admin_config.as_deref() {
        Some(c) => c,
        None => default_admin_config(),
    };
    let max_conversations = admin_config
        .get_value(
            "usage_quotas.max_active_conversations",
            ConfigLookupScope::user(&user_id_str, &tenant_id.to_string()),
        )
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_MAX_ACTIVE_CONVERSATIONS);

    let current_count = resources
        .common
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

    // Verify group membership when caller asks to attach a group_id —
    // a user can only create a conversation scoped to a group they belong to,
    // or one they are the human coach of.
    if let Some(gid) = request.group_id.as_deref() {
        verify_group_membership(&resources, gid, auth.user_id, tenant_id).await?;
    }

    let result = create_conversation_row(
        resources.common.repos.chat.as_ref(),
        &user_id_str,
        tenant_id,
        &request.title,
        request.model.as_deref(),
        request.coach_id.as_deref(),
        request.group_id.as_deref(),
    )
    .await?;

    // Bump the per-coach use_count + last_used_at when a conversation is
    // opened with a coach attached. Audit (2026-05-07) showed every coach
    // stuck at "0 uses" because nothing on the chat path called
    // record_usage even though the field is shown in the Coaches UI.
    if let Some(coach_id) = request.coach_id.as_deref() {
        record_coach_usage_best_effort(&resources, coach_id, auth.user_id, tenant_id).await;
    }

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

/// List every conversation the caller takes part in — whatever surface opened
/// it — newest activity first, one page at a time.
///
/// `total` is the caller's real count, so a client knows whether a "load more"
/// is owed; the page bounds are clamped server-side (see
/// [`ListConversationsQuery::bounded`]).
pub async fn list_conversations(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<ListConversationsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let (limit, offset) = query.bounded();

    let page = resources
        .common
        .repos
        .chat
        .list_conversations(&auth.user_id.to_string(), tenant_id, limit, offset)
        .await?;

    let response = ConversationListResponse {
        conversations: page
            .items
            .into_iter()
            .map(|c| ConversationSummaryResponse {
                id: c.id,
                title: c.title,
                model: c.model,
                message_count: c.message_count,
                total_tokens: c.total_tokens,
                coach_id: c.coach_id,
                coach_handle: c.coach_handle,
                coach_title: c.coach_title,
                group_id: c.group_id,
                group_name: c.group_name,
                channel_type: c.channel_type,
                last_message: c.last_message.map(|m| LastMessageResponse {
                    preview: preview_text(&m.content_head),
                    role: m.role,
                    created_at: m.created_at,
                }),
                unread_count: c.unread_count,
                created_at: c.created_at,
                updated_at: c.updated_at,
            })
            .collect(),
        total: page.total,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Fetch a single conversation by id (scoped to the caller's tenant).
pub async fn get_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    let conv = resources
        .common
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
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Json(request): Json<UpdateConversationRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    let updated = resources
        .common
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
        .common
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
///
/// Deletion is the one conversation write reserved to the owner: a
/// participant who was added to the thread is refused with 403 rather than
/// the 404 a stranger gets, because for them the conversation does exist.
pub async fn delete_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();

    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    if conv.user_id != user_id_str {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only the conversation's owner can delete it",
        ));
    }

    let deleted = resources
        .common
        .repos
        .chat
        .delete_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?;

    if !deleted {
        return Err(AppError::not_found("Conversation not found"));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// List all messages in a conversation after verifying the caller is a participant.
///
/// Reading the thread is reading it: the caller's read marker advances to the
/// newest turn, so the list row's unread badge clears on the next list. The
/// marker write is best-effort — the rows are what the caller asked for, and a
/// marker that failed to move costs one spurious badge, not the transcript.
pub async fn get_messages(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    // Verify the caller participates in this conversation
    resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let user_id_str = auth.user_id.to_string();
    let messages = resources
        .common
        .repos
        .chat
        .get_messages(&conversation_id, &user_id_str, tenant_id)
        .await?;

    // The stored locale rather than a per-turn detection: a chart written three
    // weeks ago should not relabel its axis because the athlete's last message
    // happened to be in another language.
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), auth.user_id).await;

    let messages_list: Vec<MessageResponse> = messages
        .into_iter()
        .map(|m| {
            let blocks = resolve_stored_blocks(m.content_blocks.as_deref(), &locale);
            MessageResponse {
                id: m.id,
                role: m.role,
                content: m.content,
                token_count: m.token_count,
                scene_blocks: blocks.scene_blocks,
                finish_reason: m.finish_reason,
                actions: blocks.actions,
                created_at: m.created_at,
            }
        })
        .collect();

    match resources
        .common
        .repos
        .chat
        .mark_conversation_read(&conversation_id, &user_id_str, tenant_id, None)
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            conversation_id = %conversation_id,
            "read marker did not advance for a participant who just read the thread"
        ),
        Err(e) => tracing::warn!(
            conversation_id = %conversation_id,
            error = %e,
            "read marker could not advance after the thread was read"
        ),
    }

    // The caller's own thumbs up/down feedback for this conversation, so the
    // client re-renders the rating state (and any saved reason) after a reload.
    let feedback: Vec<MessageFeedbackEntry> = resources
        .common
        .repos
        .chat
        .get_conversation_feedback(&conversation_id, &user_id_str, tenant_id)
        .await?
        .into_iter()
        .map(|f| MessageFeedbackEntry {
            message_id: f.message_id,
            rating: f.rating,
            comment: f.comment,
        })
        .collect();

    let response = MessagesListResponse {
        messages: messages_list,
        feedback,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
