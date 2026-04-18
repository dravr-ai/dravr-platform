// ABOUTME: Turn-level input and output types for the unified chat pipeline
// ABOUTME: DispatchResult carries usage metadata needed for cost tracking and quota enforcement
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn input and output types for [`super::run`].

use pierre_database::database::{ConversationRecord, MessageRecord};

use crate::llm::TokenUsage;
use crate::models::TenantId;

/// Result of creating a new conversation, including the validated model.
pub struct CreateConversationResult {
    /// The created conversation record.
    pub conversation: ConversationRecord,
}

/// Result of persisting a user message and resolving the parent conversation.
pub struct UserMessageResult {
    /// The persisted user message.
    pub message: MessageRecord,
    /// The conversation record (for model / `coach_id` access during dispatch).
    pub conversation: ConversationRecord,
}

/// Input to a single pipeline turn.
///
/// `conversation_tenant_id` is used for conversation/message DB lookups.
/// `tool_tenant_id` is used for tool execution (OAuth, activities, etc.).
/// These may differ when a messaging user belongs to a different tenant
/// than the bot that owns the channel webhook.
#[derive(Debug, Clone)]
pub struct TurnInput {
    /// Identifier of the conversation the turn is appending to.
    pub conversation_id: String,
    /// User UUID as string.
    pub user_id: String,
    /// Tenant used for conversation and message DB lookups.
    pub conversation_tenant_id: TenantId,
    /// Tenant used for tool execution (OAuth credentials, provider APIs).
    pub tool_tenant_id: TenantId,
    /// Raw user message content.
    pub content: String,
    /// BCP-47 short locale resolved upstream (messaging ingress / web chat).
    ///
    /// `None` means the caller did not plumb a locale and pipeline stages
    /// should fall back to the registry's `DEFAULT_LOCALE`. Web chat may
    /// leave this `None` until the web-side locale picker lands.
    pub locale: Option<String>,
}

/// Read-only view of the current turn available to hooks and stages.
///
/// Mutable pipeline state (history, `llm_messages`, system prompt) lives in
/// local variables inside [`super::run`] — this context is what hooks see.
#[derive(Debug, Clone)]
pub struct TurnContext {
    /// Echoes `TurnInput::conversation_id`.
    pub conversation_id: String,
    /// Echoes `TurnInput::user_id`.
    pub user_id: String,
    /// Echoes `TurnInput::conversation_tenant_id`.
    pub conversation_tenant_id: TenantId,
    /// Echoes `TurnInput::tool_tenant_id`.
    pub tool_tenant_id: TenantId,
    /// Echoes `TurnInput::locale`; `None` triggers the registry default.
    pub locale: Option<String>,
}

impl TurnContext {
    /// Derive a context from an input.
    #[must_use]
    pub fn from_input(input: &TurnInput) -> Self {
        Self {
            conversation_id: input.conversation_id.clone(),
            user_id: input.user_id.clone(),
            conversation_tenant_id: input.conversation_tenant_id,
            tool_tenant_id: input.tool_tenant_id,
            locale: input.locale.clone(),
        }
    }
}

/// Result of dispatching a single turn.
///
/// Contains the final assistant text along with the persisted message
/// records and usage metadata channel adapters need to render a response
/// (web chat returns the records in its HTTP body; messaging reads usage
/// fields for cost recording and ignores the records).
#[derive(Debug, Clone)]
pub struct DispatchResult {
    /// Final text content after all post-processing stages
    /// (`apply_text_guardrails`, `apply_claim_verification`, and any
    /// `ResponsePostProcess` hook).
    pub content: String,
    /// Token usage reported by the LLM provider, if available.
    ///
    /// CLI-based providers may return `None`, in which case channel
    /// adapters fall back to character-based estimation when recording usage.
    pub usage: Option<TokenUsage>,
    /// Number of tool calls made during the turn's tool loop.
    pub tool_calls_count: u32,
    /// Finish reason from the LLM provider.
    pub finish_reason: Option<String>,
    /// Formatted activity list captured from the tool loop, surfaced as
    /// a separate frontend-rendered panel in the web UI.
    pub activity_list: Option<String>,
    /// Model identifier actually used on this turn — may differ from the
    /// conversation's stored model when [`super::ModelPolicy::OverrideWithEnv`]
    /// is in effect.
    pub model: String,
    /// Name of the LLM provider used (e.g. "gemini", "groq", `"copilot_headless"`).
    pub provider_name: String,
    /// Persisted user message record.
    pub user_message: MessageRecord,
    /// Persisted assistant message record.
    pub assistant_message: MessageRecord,
    /// Conversation record reloaded after the assistant message landed —
    /// carries the updated `updated_at` / `summary` fields.
    pub conversation: ConversationRecord,
}
