// ABOUTME: Conversation and message persistence stages — create, verify ownership, append, retrieve
// ABOUTME: Extracted from services/chat_orchestration.rs persistence helpers (2026-04-16)
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
//! 3. Get conversation history — retrieves all messages for the
//!    conversation, ordered oldest-first, for LLM context assembly.
//! 4. Persist assistant response — appends the LLM reply with full
//!    token usage and model metadata, then re-reads the conversation
//!    record so the caller can access updated `updated_at` / `summary`
//!    fields.

use pierre_core::models::AddMessageParams;
use pierre_database::database::repositories::ChatRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};

use crate::config::LlmProviderType;
use crate::errors::{AppError, AppResult};
use crate::models::TenantId;
use crate::services::chat_pipeline::turn::{CreateConversationResult, UserMessageResult};

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
) -> AppResult<CreateConversationResult> {
    let model = match requested_model {
        Some(m) => m.to_owned(),
        None => LlmProviderType::model_from_env().ok_or_else(|| {
            AppError::config("No model specified and PIERRE_LLM_MODEL environment variable not set")
        })?,
    };

    let conversation = database
        .create_conversation(user_id, tenant_id, title, &model, coach_id)
        .await?;

    Ok(CreateConversationResult { conversation })
}

/// Verify conversation ownership and persist the user message.
///
/// Business rules:
/// - Conversation must exist and belong to the user/tenant
/// - Message is persisted before LLM dispatch (crash-safe)
/// - Returns both message and conversation (for model/prompt access in LLM step)
///
/// # Errors
///
/// Returns `AppError::NotFound` if the conversation does not exist or
/// belongs to another user. Returns database errors on message persistence
/// failure.
pub async fn persist_user_message(
    database: &dyn ChatRepository,
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
        conversation_id,
        user_id,
        role: "user",
        content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
    };
    let message = database.add_message(&user_msg_params).await?;

    Ok(UserMessageResult {
        message,
        conversation,
    })
}

/// Get conversation history for LLM context building.
///
/// Returns all messages in the conversation for the given user.
///
/// # Errors
///
/// Returns database errors on message retrieval failure.
pub async fn get_conversation_history(
    database: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
) -> AppResult<Vec<MessageRecord>> {
    database.get_messages(conversation_id, user_id).await
}

/// Persist the assistant's response message.
///
/// Called after LLM dispatch + tool execution completes. Returns the
/// persisted message record and updated conversation so the caller can
/// observe `updated_at` / `summary` fields that moved as a result.
///
/// # Errors
///
/// Returns `AppError::Internal` if the conversation cannot be retrieved
/// after saving. Returns database errors on message persistence failure.
pub async fn persist_assistant_response(
    database: &dyn ChatRepository,
    params: &AddMessageParams<'_>,
    tenant_id: TenantId,
) -> AppResult<(MessageRecord, ConversationRecord)> {
    let message = database.add_message(params).await?;

    let conversation = database
        .get_conversation(params.conversation_id, params.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("Failed to get updated conversation"))?;

    Ok((message, conversation))
}
