// ABOUTME: GET /api/chat/conversations/{id}/messages and /topics — a thread read in windows from the newest back, and its topics
// ABOUTME: Participant-gated; the personal thread never rotates, so neither read serves it whole (carnet#840)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reading a thread.
//!
//! The athlete's personal thread is one conversation for good: "+" and
//! `/reset` open topics inside it rather than new threads, and the messaging
//! DM writes into it too. Served whole, it would grow without bound, so the
//! transcript comes in windows: the newest by default — what a client gets
//! without asking for anything else — and the window before a row the client
//! already holds when it scrolls back. History lists the thread's topics, each
//! titled with the athlete's first words in it.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::turn_watchdog;
use chrono::Utc;
use pierre_chat_pipeline::turn_stop::{awaiting_question, recent_rows};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_database::repositories::ChatRepository;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::ai_scope;
use pierre_services::locale::resolve_user_locale;
use pierre_tool_runtime::derived_content;

use super::common::get_tenant_id;
use super::dto::{
    preview_text, resolve_stored_blocks, AwaitingReply, MessageFeedbackEntry, MessageResponse,
    MessagesListResponse, TopicResponse, TopicsListResponse,
};

/// Messages served when the caller names no window size.
const DEFAULT_MESSAGE_LIMIT: i64 = 100;

/// Upper bound on one window.
const MAX_MESSAGE_LIMIT: i64 = 200;

/// Most topics one History list reads.
const MAX_TOPICS: i64 = 200;

/// Query parameters for a window of the transcript.
#[derive(Debug, Deserialize)]
pub(super) struct MessagesQuery {
    /// Messages to return (clamped to `1..=200`; 100 when absent).
    #[serde(default)]
    pub limit: Option<i64>,
    /// Id of the oldest message the caller already holds: the window then
    /// holds the messages before it. Absent for the newest window.
    #[serde(default)]
    pub before: Option<String>,
}

/// Read one window of a conversation's messages, as a participant.
///
/// Reading the newest window is reading the thread: the caller's read marker
/// advances to the newest turn, so the list row's unread badge clears on the
/// next list. An older window moves nothing. The marker write is
/// best-effort — the rows are what the caller asked for, and a marker that
/// failed to move costs one spurious badge, not the transcript.
pub(super) async fn get_messages(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();
    let chat = resources.common.repos.chat.as_ref();

    // Verify the caller participates in this conversation
    chat.get_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    // A thread opened from a first-party-only activity is about it
    // throughout (carnet#769).
    derived_content::refuse_withheld_thread(
        resources.as_ref(),
        &tenant_id,
        auth.user_id,
        &conversation_id,
    )
    .await?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGE_LIMIT)
        .clamp(1, MAX_MESSAGE_LIMIT);
    let before = query.before.as_deref().filter(|id| !id.is_empty());
    let page = chat
        .get_message_page(&conversation_id, &user_id_str, tenant_id, before, limit)
        .await?;
    let mut messages = page.messages;
    // A row derived from first-party-only data is withheld from an external
    // caller; the rest of the window is served (carnet#769).
    ai_scope::retain_admitted(&mut messages, |m| m.transport_policy);

    // The stored locale rather than a per-turn detection: a chart written three
    // weeks ago should not relabel its axis because the athlete's last message
    // happened to be in another language.
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), auth.user_id).await;

    let messages_list: Vec<MessageResponse> = messages
        .into_iter()
        .map(|m| {
            let blocks = resolve_stored_blocks(m.content_blocks.as_deref(), &locale);
            MessageResponse {
                id: m.id,
                role: m.role,
                content: m.content,
                token_count: m.token_count,
                scene_blocks: blocks.scene_blocks,
                finish_reason: m.finish_reason,
                actions: blocks.actions,
                channel_type: m.channel_type,
                created_at: m.created_at,
            }
        })
        .collect();

    if before.is_none() {
        match chat
            .mark_conversation_read(&conversation_id, &user_id_str, tenant_id, None)
            .await
        {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                conversation_id = %conversation_id,
                "read marker did not advance for a participant who just read the thread"
            ),
            Err(e) => tracing::warn!(
                conversation_id = %conversation_id,
                error = %e,
                "read marker could not advance after the thread was read"
            ),
        }
    }

    // The caller's own thumbs up/down feedback for this conversation, so the
    // client re-renders the rating state (and any saved reason) after a reload.
    let feedback: Vec<MessageFeedbackEntry> = chat
        .get_conversation_feedback(&conversation_id, &user_id_str, tenant_id)
        .await?
        .into_iter()
        .map(|f| MessageFeedbackEntry {
            message_id: f.message_id,
            rating: f.rating,
            comment: f.comment,
        })
        .collect();

    let awaiting_reply = awaiting_reply(
        &resources,
        chat,
        &conversation_id,
        &user_id_str,
        tenant_id,
        before,
    )
    .await?;

    let response = MessagesListResponse {
        messages: messages_list,
        feedback,
        has_more: page.has_more,
        awaiting_reply,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// The caller's question a turn is still working on, if any.
///
/// Only the newest window (`before` absent) says whether a reply is coming:
/// an older one is history the client scrolled back to, and answers `None`.
///
/// Read from the same tail of the conversation `POST …/stop` reads, not from
/// the window being served — a window of one row may not reach the question —
/// so a client told a reply is coming can always stop that turn, and the two
/// never disagree about which question is open. Only the caller's own question
/// counts: in a shared conversation, another participant's running turn is
/// theirs to wait on.
///
/// Aged against the turn watchdog ([`turn_watchdog`]), the one figure the
/// server names as the longest a healthy turn can run. An in-app turn is not
/// cut at it — only the messaging path enforces it — but nothing a live turn
/// waits on lasts longer, so an unanswered question past it is a turn that died
/// without writing a row, and reporting it would keep a client waiting on
/// nothing.
async fn awaiting_reply(
    resources: &ServerContext,
    chat: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    before: Option<&str>,
) -> Result<Option<AwaitingReply>, AppError> {
    if before.is_some() {
        return Ok(None);
    }
    let rows = recent_rows(chat, conversation_id, user_id, tenant_id).await?;
    let ceiling = turn_watchdog(&resources.common.config.timeouts);
    Ok(
        awaiting_question(&rows, user_id, Utc::now(), ceiling).map(|question| AwaitingReply {
            question_id: question.id.clone(),
            asked_at: question.created_at.clone(),
        }),
    )
}

/// List a conversation's topics, newest first, as a participant: one per
/// divider, plus the thread's opening. History draws them as the stretches of
/// the personal thread, each titled with the athlete's first words in it.
pub(super) async fn list_topics(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();
    let chat = resources.common.repos.chat.as_ref();

    chat.get_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    derived_content::refuse_withheld_thread(
        resources.as_ref(),
        &tenant_id,
        auth.user_id,
        &conversation_id,
    )
    .await?;

    let topics = chat
        .list_topics(&conversation_id, &user_id_str, tenant_id, MAX_TOPICS)
        .await?
        .into_iter()
        .map(|topic| TopicResponse {
            divider_id: topic.divider_id,
            started_at: topic.started_at,
            // The athlete's own words, so nothing first-party-only titles a
            // topic; laid out as one plain line, like a list row's preview.
            title: topic
                .first_words
                .map(|words| preview_text(&words))
                .filter(|title| !title.is_empty()),
        })
        .collect();

    Ok((StatusCode::OK, Json(TopicsListResponse { topics })).into_response())
}
