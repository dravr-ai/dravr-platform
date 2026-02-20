// ABOUTME: Chat conversation and message record types for database persistence
// ABOUTME: DTOs for multi-tenant chat conversations with LLM model tracking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

/// Database representation of a chat conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    /// Unique conversation ID
    pub id: String,
    /// User ID who owns the conversation
    pub user_id: String,
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: String,
    /// Conversation title (auto-generated or user-defined)
    pub title: String,
    /// LLM model used for this conversation
    pub model: String,
    /// Optional system prompt for the conversation
    pub system_prompt: Option<String>,
    /// Total tokens used in this conversation
    pub total_tokens: i64,
    /// When the conversation was created (ISO 8601)
    pub created_at: String,
    /// When the conversation was last updated (ISO 8601)
    pub updated_at: String,
}

/// Database representation of a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    /// Unique message ID
    pub id: String,
    /// Conversation ID this message belongs to
    pub conversation_id: String,
    /// Role of the message sender (system, user, assistant)
    pub role: String,
    /// Message content
    pub content: String,
    /// Token count for this message (completion tokens for assistant messages)
    pub token_count: Option<i64>,
    /// Prompt tokens used to generate this message
    pub prompt_tokens: Option<i64>,
    /// LLM model used to generate this message
    pub model: Option<String>,
    /// Finish reason for assistant messages
    pub finish_reason: Option<String>,
    /// When the message was created (ISO 8601)
    pub created_at: String,
}

/// Summary of a conversation for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// Conversation ID
    pub id: String,
    /// Conversation title
    pub title: String,
    /// LLM model used
    pub model: String,
    /// Number of messages in the conversation
    pub message_count: i64,
    /// Total tokens used
    pub total_tokens: i64,
    /// When the conversation was created
    pub created_at: String,
    /// When the conversation was last updated
    pub updated_at: String,
}

/// Parameters for adding a message to a conversation
pub struct AddMessageParams<'a> {
    /// Conversation to add the message to
    pub conversation_id: &'a str,
    /// User who owns the conversation
    pub user_id: &'a str,
    /// Role of the message sender (`user`, `assistant`, `system`)
    pub role: &'a str,
    /// Message content
    pub content: &'a str,
    /// Completion token count (for assistant messages)
    pub token_count: Option<u32>,
    /// Reason the LLM stopped generating (e.g. `stop`, `length`)
    pub finish_reason: Option<&'a str>,
    /// Prompt token count for this interaction
    pub prompt_tokens: Option<u32>,
    /// LLM model identifier used for this message
    pub model: Option<&'a str>,
}
