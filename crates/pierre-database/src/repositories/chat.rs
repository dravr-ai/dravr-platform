// ABOUTME: Repository trait definitions for the chat conversation persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::AddMessageParams;
use pierre_core::models::{ConversationRecord, ConversationSummary};
use pierre_core::models::{MessageRecord, TenantId};

/// Chat conversation and message management repository
#[async_trait]
pub trait ChatRepository: Send + Sync {
    /// Create a new chat conversation.
    ///
    /// `coach_id` references a coach in the `coaches` table; the coach's
    /// `system_prompt` is the canonical persona source and is resolved at
    /// runtime via [`CoachesRepository::get_coach_runtime_context`].
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        coach_id: Option<&str>,
        group_id: Option<&str>,
    ) -> AppResult<ConversationRecord>;
    /// Get a conversation by ID with user/tenant isolation
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>>;
    /// List conversations for a user with pagination
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>>;
    /// Update conversation title
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool>;
    /// Delete a conversation and its messages
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Add a message to a conversation (verifies user owns the conversation)
    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord>;
    /// Get all messages for a conversation (verifies user owns the conversation)
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageRecord>>;
    /// Get recent messages for a conversation (verifies user owns the conversation)
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>>;
    /// Get message count for a conversation (verifies user owns the conversation)
    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;
    /// Count total conversations for a user in a tenant
    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64>;
    /// Delete all conversations for a user
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;

    /// Get recently updated conversations across all tenants (admin view)
    ///
    /// Returns the last `limit` conversations ordered by `updated_at` descending.
    /// Includes the associated `user_id` for display. Used by the admin activity dashboard.
    async fn get_recent_conversations_admin(
        &self,
        limit: i64,
    ) -> AppResult<Vec<ConversationRecord>>;

    /// Count conversations updated since a given timestamp (admin view, cross-tenant)
    async fn count_active_conversations_since(&self, since: &str) -> AppResult<i64>;

    /// Attach a coach session id to an existing conversation row (Tier 4
    /// cross-channel continuity). Tenant-scoped; returns `false` if the
    /// conversation does not exist.
    async fn set_conversation_session_id(
        &self,
        conversation_id: &str,
        session_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Attach a coaching group id to an existing conversation row.
    ///
    /// Used by the messaging-ingress auto-bind path to retrofit
    /// `chat_conversations.group_id` onto a conversation that pre-dates
    /// the channel/group binding (legacy sessions, or a freshly forged
    /// self-heal conversation). Tenant-scoped; returns `false` if the
    /// conversation does not exist.
    async fn set_conversation_group_id(
        &self,
        conversation_id: &str,
        group_id: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
}
