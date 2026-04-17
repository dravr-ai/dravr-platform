// ABOUTME: Thin shim over chat_pipeline::stages::persistence::create_conversation
// ABOUTME: Web chat and messaging ingress use this path to create new conversations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Conversation creation shim.
//!
//! The unified turn dispatch lives in [`crate::services::chat_pipeline::run`];
//! this module retains only the conversation-creation entry point that
//! both `routes::chat::create_conversation` and
//! `services::messaging_ingress` call when opening a new conversation.
//! Message persistence, history retrieval, and LLM dispatch all live in
//! [`crate::services::chat_pipeline`].

use crate::errors::AppResult;
use crate::models::TenantId;
use crate::services::chat_pipeline::stages::persistence::create_conversation as pipeline_create_conversation;
use crate::services::chat_pipeline::turn::CreateConversationResult;
use pierre_database::database::repositories::ChatRepository;

/// Validate the model and create a conversation.
///
/// Thin shim over
/// [`crate::services::chat_pipeline::stages::persistence::create_conversation`]
/// so existing callers that reference `chat_orchestration::create_conversation`
/// continue to resolve.
///
/// # Errors
///
/// See
/// [`crate::services::chat_pipeline::stages::persistence::create_conversation`].
pub async fn create_conversation(
    database: &dyn ChatRepository,
    user_id: &str,
    tenant_id: TenantId,
    title: &str,
    requested_model: Option<&str>,
    coach_id: Option<&str>,
) -> AppResult<CreateConversationResult> {
    pipeline_create_conversation(
        database,
        user_id,
        tenant_id,
        title,
        requested_model,
        coach_id,
    )
    .await
}
