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
//! - [`chat::personal`] — the caller's personal thread and the topics "+" opens in it
//! - [`chat::subject_thread`] — a group's coach's thread about one athlete, opened or reopened
//! - [`chat::subject_plan`] — in that thread, the coach's draft of the athlete's plan: read, publish, discard
//! - [`chat::read_marker`] — the caller's read marker: mark read / mark unread
//! - [`chat::starters`] — the use-case starters an empty thread offers
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
mod messages;
mod participants;
mod personal;
mod read_marker;
mod send_message;
mod starters;
mod stop_turn;
mod subject_plan;
mod subject_thread;
mod turn_response;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::mcp::resources::ServerContext;
use crate::services::chat_verdicts;

pub(crate) use common::get_tenant_id;
pub use dto::{
    preview_text, AddParticipantRequest, ChatMessageAction, ConversationListResponse,
    ConversationResponse, ConversationSummaryResponse, CreateConversationRequest, FeedbackRating,
    LastMessageResponse, ListConversationsQuery, MessageActionsResponse, MessageFeedbackEntry,
    MessageResponse, MessagesListResponse, OpenTopicRequest, OpenTopicResponse,
    ParticipantListResponse, ParticipantResponse, PersonalThreadResponse, SendMessageRequest,
    StarterResponse, StartersResponse, UpdateConversationRequest, UpsertFeedbackRequest,
    MAX_LIST_LIMIT,
};
pub(crate) use personal::welcome_onboarding_lead;
pub use read_marker::MarkReadRequest;
pub(crate) use send_message::client_surface;
pub use stop_turn::StopTurnResponse;
pub use turn_response::{
    AssistantResponse, NoticeResponse, ReplyBlockResponse, TurnResponse, TurnTelemetryResponse,
    VerdictChipResponse,
};

/// Chat routes handler.
#[derive(Debug)]
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
            // The caller's one personal thread, resolved on the server, and
            // the topics "+" opens inside it (carnet#840)
            .route("/api/chat/personal", get(personal::get_personal_thread))
            .route(
                "/api/chat/personal/topics",
                post(personal::open_personal_topic),
            )
            // A group's human coach opens (or reopens) their thread about one
            // athlete they coach (carnet#1070)
            .route(
                "/api/chat/athletes/{athlete_user_id}/thread",
                post(subject_thread::open_subject_thread),
            )
            // In that thread, the coach's draft of the athlete's plan: read
            // it beside the live plan, publish it, or throw it away
            // (carnet#1069)
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan",
                get(subject_plan::get_subject_plan),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/publish",
                post(subject_plan::publish_subject_plan),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft",
                delete(subject_plan::discard_subject_plan_draft),
            )
            // The use-case starters an empty thread offers, ranked from the
            // athlete's state (carnet#829)
            .route("/api/chat/starters", get(starters::list_starters))
            // Participants: who can read and post in the conversation
            .route(
                "/api/chat/conversations/{conversation_id}/participants",
                get(participants::list_participants).post(participants::add_participant),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/participants/{user_id}",
                delete(participants::remove_participant),
            )
            // The caller's read marker: mark read (advance) / mark unread (clear)
            .route(
                "/api/chat/conversations/{conversation_id}/read",
                post(read_marker::mark_read).delete(read_marker::mark_unread),
            )
            // Messages, read in windows from the newest back, and the topics
            // the dividers open (carnet#840)
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                get(messages::get_messages),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/topics",
                get(messages::list_topics),
            )
            // Shared group room transcript (membership-gated, consent-filtered)
            .route(
                "/api/chat/groups/{group_id}/transcript",
                get(group_transcript::get_group_transcript),
            )
            // POST messages with MCP tool support — an event stream when the
            // request's Accept header names text/event-stream, one JSON
            // document otherwise
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                post(send_message::send_message),
            )
            // The athlete stops the turn answering their latest message
            .route(
                "/api/chat/conversations/{conversation_id}/stop",
                post(stop_turn::stop_turn),
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
