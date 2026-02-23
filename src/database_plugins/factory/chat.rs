// ABOUTME: Chat repository dispatch for the database factory
// ABOUTME: Delegates ChatRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database::{ConversationRecord, ConversationSummary, MessageRecord};
use crate::database_plugins::ChatRepository;
use crate::errors::AppResult;
use async_trait::async_trait;
use pierre_core::models::{AddMessageParams, TenantId};

#[async_trait]
impl ChatRepository for Database {
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        system_prompt: Option<&str>,
    ) -> AppResult<ConversationRecord> {
        match self {
            Self::SQLite(db) => {
                db.chat_create_conversation_impl(user_id, tenant_id, title, model, system_prompt)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.create_conversation(user_id, tenant_id, title, model, system_prompt)
                    .await
            }
        }
    }
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>> {
        match self {
            Self::SQLite(db) => {
                db.chat_get_conversation_impl(conversation_id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_conversation(conversation_id, user_id, tenant_id)
                    .await
            }
        }
    }
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>> {
        match self {
            Self::SQLite(db) => {
                db.chat_list_conversations_impl(user_id, tenant_id, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.list_conversations(user_id, tenant_id, limit, offset)
                    .await
            }
        }
    }
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.chat_update_conversation_title_impl(conversation_id, user_id, tenant_id, title)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_conversation_title(conversation_id, user_id, tenant_id, title)
                    .await
            }
        }
    }
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.chat_delete_conversation_impl(conversation_id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.delete_conversation(conversation_id, user_id, tenant_id)
                    .await
            }
        }
    }
    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord> {
        match self {
            Self::SQLite(db) => db.chat_add_message_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.add_message(params).await,
        }
    }
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> AppResult<Vec<MessageRecord>> {
        match self {
            Self::SQLite(db) => db.chat_get_messages_impl(conversation_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_messages(conversation_id, user_id).await,
        }
    }
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>> {
        match self {
            Self::SQLite(db) => {
                db.chat_get_recent_messages_impl(conversation_id, user_id, limit)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_recent_messages(conversation_id, user_id, limit)
                    .await
            }
        }
    }
    async fn get_message_count(&self, conversation_id: &str, user_id: &str) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => {
                db.chat_get_message_count_impl(conversation_id, user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_message_count(conversation_id, user_id).await,
        }
    }
    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => db.chat_count_conversations_impl(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count_conversations(user_id, tenant_id).await,
        }
    }
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => {
                db.chat_delete_all_user_conversations_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_all_user_conversations(user_id, tenant_id).await,
        }
    }
}
