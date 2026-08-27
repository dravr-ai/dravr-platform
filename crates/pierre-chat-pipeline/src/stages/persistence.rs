// ABOUTME: Conversation and message persistence stages — create, verify ownership, append, retrieve
// ABOUTME: Single entry point both routes/chat and messaging_ingress use to open a new conversation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Conversation and message persistence.
//!
//! Four responsibilities:
//!
//! 1. Create conversation — validates the model selection against
//!    `PIERRE_LLM_MODEL` env fallback, persists the new conversation record.
//! 2. Persist user message — verifies conversation ownership (tenant +
//!    user), persists the user turn before LLM dispatch (crash-safe: a
//!    server failure mid-dispatch still leaves the user's question visible
//!    on reload).
//! 3. Get conversation history — retrieves the recent messages for the
//!    conversation, ordered oldest-first, for LLM context assembly. Slash
//!    command rows are left out here: they are transcript, never prompt.
//! 4. Persist assistant response — appends the LLM reply with full
//!    token usage and model metadata, then re-reads the conversation
//!    record so the caller can access updated `updated_at` / `summary`
//!    fields.
//!
//! Both message writers also fan the row out to the group's shared room
//! transcript (`group_transcript_entries`) when the conversation is
//! group-bound, so every surface's group turns land in one queryable,
//! consent-gated room view — and both advance the participant's read
//! marker past the row they wrote, so an athlete's own turn never counts
//! as unread for them. One rule for web, mobile and messaging.

use pierre_core::models::AddMessageParams;
use pierre_database::database::repositories::ChatRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::repositories::CoachingGroupRepository;

use crate::turn::{CreateConversationResult, UserMessageResult};
use pierre_config::environment::LlmProviderType;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{NewGroupTranscriptEntry, TranscriptSpeaker};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use tracing::warn;

/// Advance the participant's read marker past a row they just wrote or were
/// just shown. Best-effort: the row is already durable, and a marker that
/// failed to move costs one spurious unread badge, not the turn.
async fn advance_read_marker(
    database: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    message_id: &str,
) {
    match database
        .mark_conversation_read(conversation_id, user_id, tenant_id, Some(message_id))
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!(
            conversation_id,
            message_id, "read marker did not advance: the row is not visible to its own writer"
        ),
        Err(e) => {
            warn!(error = %e, conversation_id, "read marker could not advance after the turn");
        }
    }
}

/// Validate the model and create a conversation.
///
/// Business rules:
/// - Uses requested model if provided
/// - Falls back to `PIERRE_LLM_MODEL` environment variable
/// - Fails if no model can be determined
///
/// # Errors
///
/// Returns `AppError::Config` if no model is specified and
/// `PIERRE_LLM_MODEL` is not set. Returns database errors on conversation
/// creation failure.
pub async fn create_conversation(
    database: &dyn ChatRepository,
    user_id: &str,
    tenant_id: TenantId,
    title: &str,
    requested_model: Option<&str>,
    coach_id: Option<&str>,
    group_id: Option<&str>,
) -> AppResult<CreateConversationResult> {
    let model = match requested_model {
        Some(m) => m.to_owned(),
        None => LlmProviderType::model_from_env().ok_or_else(|| {
            AppError::config("No model specified and PIERRE_LLM_MODEL environment variable not set")
        })?,
    };

    let conversation = database
        .create_conversation(user_id, tenant_id, title, &model, coach_id, group_id)
        .await?;

    Ok(CreateConversationResult { conversation })
}

/// Fan a just-persisted user/assistant row out to the group's shared room
/// transcript when the conversation is group-bound.
///
/// The entry is attributed to the conversation's member (`user_id`) for both
/// speakers: a coach row is the reply that member received, so consent
/// withholding hides the pair together. Non-group conversations append
/// nothing.
///
/// # Errors
///
/// Returns database errors from the transcript append — the room transcript
/// is turn persistence, and a silently missing row would replay as a hole in
/// every member's shared view.
async fn fan_out_to_group_transcript(
    groups: &dyn CoachingGroupRepository,
    conversation: &ConversationRecord,
    tenant_id: TenantId,
    user_id: &str,
    speaker: TranscriptSpeaker,
    content: &str,
    source_message_id: &str,
) -> AppResult<()> {
    let Some(group_id) = conversation.group_id.as_deref() else {
        return Ok(());
    };
    let tenant_str = tenant_id.to_string();
    let entry = NewGroupTranscriptEntry {
        group_id,
        tenant_id: &tenant_str,
        author_user_id: parse_uuid(user_id)?,
        speaker,
        content,
        source_conversation_id: Some(&conversation.id),
        source_message_id: Some(source_message_id),
    };
    groups.append_transcript_entry(&entry).await
}

/// Verify conversation ownership and persist the user message.
///
/// Business rules:
/// - Conversation must exist and belong to the user/tenant
/// - Message is persisted before LLM dispatch (crash-safe)
/// - Group-bound conversations fan the row out to the shared room transcript
/// - Returns both message and conversation (for model/prompt access in LLM step)
///
/// # Errors
///
/// Returns `AppError::NotFound` if the conversation does not exist or
/// belongs to another user. Returns database errors on message persistence
/// failure.
pub async fn persist_user_message(
    database: &dyn ChatRepository,
    groups: &dyn CoachingGroupRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    content: &str,
) -> AppResult<UserMessageResult> {
    // Verify ownership and get conversation details
    let conversation = database
        .get_conversation(conversation_id, user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    // Persist user message before LLM dispatch
    let user_msg_params = AddMessageParams {
        tenant_id,
        conversation_id,
        user_id,
        role: "user",
        content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
        content_blocks: None,
    };
    let message = database.add_message(&user_msg_params).await?;
    advance_read_marker(database, conversation_id, user_id, tenant_id, &message.id).await;

    fan_out_to_group_transcript(
        groups,
        &conversation,
        tenant_id,
        user_id,
        TranscriptSpeaker::Member,
        content,
        &message.id,
    )
    .await?;

    Ok(UserMessageResult {
        message,
        conversation,
    })
}

/// Get the per-turn conversation history for LLM context building.
///
/// Returns the most recent `limit` messages in chronological order. The load is
/// bounded (not the full unbounded history) so a long thread doesn't build a
/// thousand-message vector before compaction trims it to the cap; `limit` is
/// derived from the compaction message cap with headroom, and is always
/// `>= max_messages` so the message-count backstop still governs the in-prompt
/// size. Read-only: the full history remains available for the UI and export
/// paths via `get_messages`.
///
/// Slash-command rows never leave this loader: a `/coach` picker or a
/// `/status` listing is the platform talking, and replayed as history it
/// would teach the model to answer in the platform's voice. The rows stay in
/// the transcript the UI reads; only the prompt is blind to them.
/// [`super::prompt_builder`] drops them by the same stamp for any caller that
/// assembles messages from rows it loaded itself.
///
/// # Errors
///
/// Returns database errors on message retrieval failure.
pub async fn get_conversation_history(
    database: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    limit: i64,
) -> AppResult<Vec<MessageRecord>> {
    let mut history = database
        .get_recent_messages(conversation_id, user_id, tenant_id, limit)
        .await?;
    history.retain(|row| !row.is_command_turn());
    Ok(history)
}

/// Persist the assistant's response message.
///
/// Called after LLM dispatch + tool execution completes. Returns the
/// persisted message record and updated conversation so the caller can
/// observe `updated_at` / `summary` fields that moved as a result.
/// Group-bound conversations fan the reply out to the shared room
/// transcript, attributed to the member it answered.
///
/// # Errors
///
/// Returns `AppError::Internal` if the conversation cannot be retrieved
/// after saving. Returns database errors on message persistence failure.
pub async fn persist_assistant_response(
    database: &dyn ChatRepository,
    groups: &dyn CoachingGroupRepository,
    params: &AddMessageParams<'_>,
    tenant_id: TenantId,
) -> AppResult<(MessageRecord, ConversationRecord)> {
    let message = database.add_message(params).await?;
    advance_read_marker(
        database,
        params.conversation_id,
        params.user_id,
        tenant_id,
        &message.id,
    )
    .await;

    let conversation = database
        .get_conversation(params.conversation_id, params.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("Failed to get updated conversation"))?;

    fan_out_to_group_transcript(
        groups,
        &conversation,
        tenant_id,
        params.user_id,
        TranscriptSpeaker::Coach,
        params.content,
        &message.id,
    )
    .await?;

    Ok((message, conversation))
}
