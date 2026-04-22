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
mod quotas;
mod send_insight;
mod send_message;
mod usage;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::errors::AppError;
use crate::llm::{ChatProvider, Tool};
use crate::mcp::resources::ServerResources;
use crate::services::chat_verdicts;
use crate::services::tool_execution::build_mcp_tools as services_build_mcp_tools;

pub use dto::{
    ChatCompletionResponse, ConversationListResponse, ConversationResponse,
    ConversationSummaryResponse, CreateConversationRequest, ListConversationsQuery,
    MessageResponse, MessagesListResponse, SendMessageRequest, UpdateConversationRequest,
};

/// Default maximum number of tool call iterations before forcing a text response.
pub(super) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Prefix used to detect insight generation requests from the frontend.
/// Must match the `INSIGHT_PROMPT_PREFIX` constant in `@pierre/chat-utils`.
pub(super) const INSIGHT_PROMPT_PREFIX: &str = "Create a shareable insight from this analysis";

/// Resolve the active `ChatProvider` from environment configuration.
///
/// Shared by `send_message` and `send_insight_message` so they report
/// the same provider name on `llm_usage` rows as the pipeline does.
pub(super) async fn get_llm_provider() -> Result<ChatProvider, AppError> {
    super::create_chat_provider().await
}

/// Build LLM tool definitions for chat-mode function calling.
pub(crate) fn build_mcp_tools() -> Tool {
    services_build_mcp_tools()
}

/// Chat routes handler.
pub struct ChatRoutes;

impl ChatRoutes {
    /// Create all chat routes.
    pub fn routes(resources: Arc<ServerResources>) -> Router {
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
            .with_state(resources)
    }
}
