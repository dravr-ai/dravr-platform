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
//! - [`chat::participants`] — who is in a conversation: list / add / remove
//! - [`chat::group_transcript`] — the shared room view of a coaching group
//! - [`chat::send_message`] — web-chat POST → unified pipeline (optional AG-UI)
//! - [`chat::turn_response`] — the wire shape of one turn's envelope
//! - [`chat::dto`] — request/response shapes shared across the handlers
//! - [`chat::common`] — auth / tenant resolution shared by every handler

mod common;
mod conversations;
mod dto;
mod feedback;
mod group_transcript;
mod participants;
mod send_message;
mod turn_response;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::mcp::resources::ServerContext;
use crate::services::chat_verdicts;

pub use dto::{
    AddParticipantRequest, ChatMessageAction, ConversationListResponse, ConversationResponse,
    ConversationSummaryResponse, CreateConversationRequest, FeedbackRating, ListConversationsQuery,
    MessageFeedbackEntry, MessageResponse, MessagesListResponse, ParticipantListResponse,
    ParticipantResponse, SendMessageRequest, UpdateConversationRequest, UpsertFeedbackRequest,
};
pub use turn_response::{
    AssistantResponse, NoticeResponse, ReplyBlockResponse, TurnResponse, TurnTelemetryResponse,
    VerdictChipResponse,
};

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
            // Participants: who can read and post in the conversation
            .route(
                "/api/chat/conversations/{conversation_id}/participants",
                get(participants::list_participants).post(participants::add_participant),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/participants/{user_id}",
                delete(participants::remove_participant),
            )
            // Messages
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                get(conversations::get_messages),
            )
            // Shared group room transcript (membership-gated, consent-filtered)
            .route(
                "/api/chat/groups/{group_id}/transcript",
                get(group_transcript::get_group_transcript),
            )
            // POST messages with MCP tool support (non-streaming)
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                post(send_message::send_message),
            )
            // Claim verdicts attached to messages in this conversation
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
