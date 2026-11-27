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
//! - [`chat::subject_plan`] — in that thread, the coach's draft of the athlete's plan: read, publish, discard, and the history of its changes
//! - [`chat::subject_session`] — in that thread, a session of the draft: built from a workout family (families, preview, save), written by hand, put back, removed or added; a date put back to the live plan
//! - [`chat::own_plan`] — the athlete's own plan, as their Plan tab reads it: the live plan, their agent's proposals over it, what « Envoyer » sends, the families, a session's preview and the history
//! - [`chat::own_plan_days`] — the athlete's own day writes on their live plan: add, family build, hand edit, restore, remove and move of one session
//! - [`chat::own_plan_send`] — the athlete's answers to their agent's proposals (accept, refuse) and « Envoyer »: its review against the provider, and the send
//! - [`chat::athlete_physiology`] — the same coach reads and sets the athlete's thresholds from Group info
//! - [`chat::read_marker`] — the caller's read marker: mark read / mark unread
//! - [`chat::starters`] — the use-case starters an empty thread offers
//! - [`chat::group_transcript`] — the shared room view of a coaching group
//! - [`chat::group_room`] — the caller's conversation in a group's room, opened when they hold none
//! - [`chat::held_invite`] — the caller joins the group whose invite the server held for them (carnet#1648)
//! - [`chat::send_message`] — web-chat POST → unified pipeline (optional AG-UI)
//! - [`chat::room_quote`] — the room entry a coach's message quotes into their athlete thread, verified
//! - [`chat::turn_response`] — the wire shape of one turn's envelope
//! - [`chat::dto`] — request/response shapes shared across the handlers
//! - [`chat::common`] — auth / tenant resolution shared by every handler

mod athlete_physiology;
mod athlete_plan;
pub(crate) mod common;
mod conversations;
mod dto;
mod feedback;
mod group_room;
mod group_transcript;
mod held_invite;
mod messages;
mod own_plan;
mod own_plan_days;
mod own_plan_send;
mod participants;
mod personal;
mod read_marker;
mod room_quote;
mod send_message;
mod starters;
mod stop_turn;
mod subject_plan;
mod subject_session;
mod subject_thread;
mod turn_response;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::mcp::resources::ServerContext;
use crate::services::chat_verdicts;

pub(crate) use common::get_tenant_id;
pub use dto::{
    preview_text, AddParticipantRequest, AwaitingReply, ChatMessageAction,
    ConversationListResponse, ConversationResponse, ConversationSummaryResponse,
    CreateConversationRequest, FeedbackRating, LastMessageResponse, ListConversationsQuery,
    MessageActionsResponse, MessageFeedbackEntry, MessageResponse, MessagesListResponse,
    OpenTopicRequest, OpenTopicResponse, ParticipantListResponse, ParticipantResponse,
    PersonalThreadResponse, SendMessageRequest, StarterResponse, StartersResponse,
    UpdateConversationRequest, UpsertFeedbackRequest, MAX_LIST_LIMIT,
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
        let router = Router::new()
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
            // The same coach reads that athlete's live plan read-only, from
            // Group info, without opening the thread (carnet#1244)
            .route(
                "/api/chat/athletes/{athlete_user_id}/plan",
                get(athlete_plan::get_athlete_plan),
            )
            // The same coach reads that athlete's thresholds per sport from
            // Group info, and sets one through the one physiology writer
            // (carnet#1561)
            .route(
                "/api/chat/athletes/{athlete_user_id}/physiology",
                get(athlete_physiology::get_athlete_physiology),
            );
        // The coach's edit saves through the physiology writer, which
        // `tools-config` builds: without it the route does not exist and the
        // read still serves.
        #[cfg(feature = "tools-config")]
        let router = router.route(
            "/api/chat/athletes/{athlete_user_id}/physiology/{sport}/{anchor}",
            put(athlete_physiology::edit::put_athlete_physiology_anchor),
        );
        router
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
            // In the same thread, a day of that draft built from a workout
            // family: the families the thread's agent offers, the session a
            // build makes, and the build saved as the day (carnet#1515)
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/families",
                get(subject_session::get_subject_families),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/session-preview",
                post(subject_session::preview_subject_session),
            )
            // One session of the date, by its position and as the client saw
            // it: a family build, edited by hand or made a rest entry, put
            // back, removed; and a session added to the date (carnet#1714,
            // carnet#1716)
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/days/{date}/sessions",
                post(subject_session::add_subject_draft_session),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/days/{date}/{position}",
                put(subject_session::put_subject_draft_session),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/days/{date}/{position}/session",
                put(subject_session::put_subject_draft_hand_session),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/days/{date}/{position}/restore",
                post(subject_session::restore_subject_draft_session),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/days/{date}/{position}/remove",
                post(subject_session::remove_subject_draft_session),
            )
            // The coach's call on one session, « C'est mon choix », made or
            // taken off; no edit of the session (carnet#1814)
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/days/{date}/{position}/coach-call",
                put(subject_session::put_subject_draft_coach_call),
            )
            // A date of the draft put back to the live plan, whole: the
            // workspace's « Refuser » on a date the agent proposed; and the
            // changes the coach and the thread's agent made to the plan,
            // newest first (carnet#1815)
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/draft/dates/{date}/revert",
                post(subject_session::revert_subject_draft_date),
            )
            .route(
                "/api/chat/conversations/{conversation_id}/subject-plan/history",
                get(subject_plan::get_subject_plan_history),
            )
            // The athlete's own plan, as their Plan tab reads it: the live
            // plan, their agent's proposals over it and what « Envoyer »
            // would send; the families « Idées » offers, a session's preview
            // on their own anchors, and the history (carnet#1841). Its
            // clients are the web and phone Plan tab, built on this branch:
            // LIMITATION(registre#1872): `/api/me/plan` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/families` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/session-preview` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/history` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/days/{date}/sessions` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/days/{date}/{position}` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/days/{date}/{position}/session` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/days/{date}/{position}/restore` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/days/{date}/{position}/remove` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/days/{date}/{position}/move` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/proposals/dates/{date}/accept` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/proposals/dates/{date}/refuse` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/send/review` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/send` has no client until the Plan tab lands
            // LIMITATION(registre#1872): `/api/me/plan/thread` has no client until the Plan tab lands
            .route("/api/me/plan", get(own_plan::get_own_plan))
            .route("/api/me/plan/families", get(own_plan::get_own_families))
            .route(
                "/api/me/plan/session-preview",
                post(own_plan::preview_own_session),
            )
            .route("/api/me/plan/history", get(own_plan::get_own_plan_history))
            // The athlete's own day writes, live at once: one session of a
            // date added, built from a family, written by hand, put back,
            // removed or moved; the first add starts the season
            // (carnet#1841)
            .route(
                "/api/me/plan/days/{date}/sessions",
                post(own_plan_days::add_own_day_session),
            )
            .route(
                "/api/me/plan/days/{date}/{position}",
                put(own_plan_days::put_own_day_session),
            )
            .route(
                "/api/me/plan/days/{date}/{position}/session",
                put(own_plan_days::put_own_day_hand_session),
            )
            .route(
                "/api/me/plan/days/{date}/{position}/restore",
                post(own_plan_days::restore_own_day_session),
            )
            .route(
                "/api/me/plan/days/{date}/{position}/remove",
                post(own_plan_days::remove_own_day_session),
            )
            .route(
                "/api/me/plan/days/{date}/{position}/move",
                post(own_plan_days::move_own_day_session),
            )
            // The athlete's answer to what their agent proposed unasked, and
            // « Envoyer »: reviewed against the provider, then sent
            // (carnet#1841)
            .route(
                "/api/me/plan/proposals/dates/{date}/accept",
                post(own_plan_send::accept_own_proposal),
            )
            .route(
                "/api/me/plan/proposals/dates/{date}/refuse",
                post(own_plan_send::refuse_own_proposal),
            )
            .route(
                "/api/me/plan/send/review",
                post(own_plan_send::review_own_plan_send),
            )
            .route("/api/me/plan/send", post(own_plan_send::send_own_plan_route))
            // The athlete's own plan thread, their Plan tab's Dravr tab: found
            // or opened, never their personal thread (carnet#1841)
            .route("/api/me/plan/thread", post(own_plan::open_own_plan_thread))
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
            // The caller's conversation in a group's room: the one they hold,
            // else one opened now — Coach Home's "Write in the group"
            .route(
                "/api/chat/groups/{group_id}/room",
                post(group_room::open_group_room),
            )
            // The group whose invite the server held for the caller, joined
            // once the account is active (carnet#1648)
            .route("/api/chat/groups/join", post(held_invite::join_held_invite))
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
