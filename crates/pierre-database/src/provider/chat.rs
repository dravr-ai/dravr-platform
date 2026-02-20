// ABOUTME: Chat database operations covering conversations and messages
// ABOUTME: Enables ChatRepository blanket impl with focused trait bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    AddMessageParams, ConversationRecord, ConversationSummary, MessageRecord, TenantId,
};

/// Chat conversation and message database operations
#[async_trait]
pub trait ChatDbOps: Send + Sync + Clone {
    /// Create a new chat conversation
    async fn chat_create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        system_prompt: Option<&str>,
    ) -> AppResult<ConversationRecord>;

    /// Get a conversation by ID with user/tenant isolation
    async fn chat_get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>>;

    /// List conversations for a user with pagination
    async fn chat_list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>>;

    /// Update conversation title
    async fn chat_update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool>;

    /// Delete a conversation and its messages
    async fn chat_delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Add a message to a conversation (verifies user owns the conversation)
    async fn chat_add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord>;

    /// Get all messages for a conversation (verifies user owns the conversation)
    async fn chat_get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> AppResult<Vec<MessageRecord>>;

    /// Get recent messages for a conversation (verifies user owns the conversation)
    async fn chat_get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>>;

    /// Get message count for a conversation (verifies user owns the conversation)
    async fn chat_get_message_count(&self, conversation_id: &str, user_id: &str) -> AppResult<i64>;

    /// Count total conversations for a user in a tenant
    async fn chat_count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64>;

    /// Delete all conversations for a user
    async fn chat_delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;
}
