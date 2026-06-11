// ABOUTME: Axum router + shared constants for /api/chat endpoints
// ABOUTME: Handler bodies live in the chat/ submodules — this file only wires routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat routes for AI conversations.
//!
//! Per-concern split:
//!
//! - [`chat::conversations`] — CRUD on `chat_conversations` + message listing
//! - [`chat::send_message`] — web-chat POST → unified pipeline (optional AG-UI)
//! - [`chat::send_insight`] — insight-generation POST → JSON-shaped one-shot
//! - [`chat::quotas`] — pre-chat quota gate + response-header warnings
//! - [`chat::usage`] — terminal `llm_usage` summary row + counter increments
//! - [`chat::dto`] — request/response shapes shared across the handlers
//! - [`chat::common`] — auth / tenant resolution shared by every handler

mod common;
mod conversations;
mod dto;
mod feedback;
mod quotas;
mod send_insight;
mod send_message;
mod usage;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::mcp::resources::ServerContext;
use crate::services::chat_verdicts;
use pierre_core::errors::AppError;
use pierre_llm::{ChatProvider, Tool};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_tool_runtime::tool_execution::build_mcp_tools as services_build_mcp_tools;

pub use dto::{
    ChatCompletionResponse, ConversationListResponse, ConversationResponse,
    ConversationSummaryResponse, CreateConversationRequest, FeedbackRating, ListConversationsQuery,
    MessageFeedbackEntry, MessageResponse, MessagesListResponse, SendMessageRequest,
    UpdateConversationRequest, UpsertFeedbackRequest,
};

/// Default maximum number of tool call iterations before forcing a text response.
pub(super) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Prefix used to detect insight generation requests from the frontend.
/// Must match the `INSIGHT_PROMPT_PREFIX` constant in `@pierre/chat-utils`.
pub(super) const INSIGHT_PROMPT_PREFIX: &str = "Create a shareable insight from this analysis";

/// Resolve the active `ChatProvider` from the shared singleton on
/// [`ServerContext`], falling back to a per-call build if no singleton
/// is wired (test fixtures).
///
/// Shared by `send_message` and `send_insight_message` so they report
/// the same provider name on `llm_usage` rows as the pipeline does,
/// and so a single Copilot Headless subprocess + token cache services
/// every chat-side request without re-spawning per call.
pub(super) async fn get_llm_provider(
    resources: &Arc<ServerContext>,
) -> Result<Arc<ChatProvider>, AppError> {
    chat_provider_from_resources_arc(
        resources.common.chat_provider.as_ref(),
        resources.common.llm_provider.as_ref(),
    )
}

/// Build LLM tool definitions for chat-mode function calling.
pub(crate) fn build_mcp_tools(resources: &Arc<ServerContext>) -> Tool {
    services_build_mcp_tools(&resources.mcp.tool_registry)
}

/// Chat routes handler.
pub struct ChatRoutes;

impl ChatRoutes {
    /// Create all chat routes.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            // Conversation management
            .route(
                "/api/chat/conversations",
                post(conversations::create_conversation),
            )
            .route(
                "/api/chat/conversations",
                get(conversations::list_conversations),
            )
            .route(
                "/api/chat/conversations/{conversation_id}",
                get(conversations::get_conversation),
            )
            .route(
                "/api/chat/conversations/{conversation_id}",
                put(conversations::update_conversation),
            )
            .route(
                "/api/chat/conversations/{conversation_id}",
                delete(conversations::delete_conversation),
            )
            // Messages
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                get(conversations::get_messages),
            )
            // POST messages with MCP tool support (non-streaming)
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                post(send_message::send_message),
            )
            // Tier 5.5 claim verdicts attached to messages in this conversation
            .route(
                "/api/chat/conversations/{conversation_id}/verdicts",
                get(chat_verdicts::get_verdicts_handler),
            )
            // Per-message thumbs up/down feedback (upsert + toggle-off)
            .route(
                "/api/chat/conversations/{conversation_id}/messages/{message_id}/feedback",
                post(feedback::upsert_feedback).delete(feedback::delete_feedback),
            )
            .with_state(resources)
    }
}
