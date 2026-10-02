// ABOUTME: POST /api/chat/conversations/{id}/stop — the athlete ends the turn answering their own latest question
// ABOUTME: Writes the stopped notice naming that question; the running turn, on any instance, ends on reading it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The stop request.
//!
//! A turn is deliberately detached from the request that started it: a
//! dropped stream — a hidden tab, a backgrounded app, a lost network — must
//! not end it, so hanging up is never read as "stop". An explicit stop is its
//! own request, and nothing routes it to the instance holding the turn, so
//! what it leaves behind is a row the turn reads:
//! [`pierre_chat_pipeline::turn_stop`] holds the turn's half and the row
//! vocabulary both halves share.
//!
//! The request names no turn. The client cannot: a phone reads the turn's
//! body only when it is complete, so nothing the server says mid-turn reaches
//! it. The server resolves the turn instead — the newest question the caller
//! wrote in this conversation, recognised by the author entry the turn
//! stored it with — and stops it only while that question has neither a
//! reply nor a stop. Another participant's question is never the caller's,
//! so nobody can stop a turn they did not start.
//!
//! What the athlete sees after stopping is what is on file: their question,
//! then the notice. The text the model had streamed is discarded rather than
//! kept as a partial reply. It is a draft: the verification, guardian and
//! response-boundary stages that decide what a reply may say run after
//! generation, so a stored fragment would be coaching text none of them read.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use pierre_chat_pipeline::stages::persistence::persist_assistant_response;
use pierre_chat_pipeline::turn_stop::{
    latest_question_by, question_state, recent_rows, stop_marker, QuestionState,
};
use pierre_contremaitre::messaging_strings::KEY_TURN_STOPPED;
use pierre_core::errors::AppError;
use pierre_core::models::{AddMessageParams, STOPPED_TURN_FINISH_REASON};
use pierre_middleware::AuthenticatedUser;
use pierre_services::locale::resolve_user_locale;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::mcp::resources::ServerContext;

use super::common::get_tenant_id;

/// Answer of `POST …/stop`.
#[derive(Debug, Serialize, Deserialize)]
pub struct StopTurnResponse {
    /// `true` when the caller's unanswered question was closed with the
    /// stopped notice. `false` when there was nothing of theirs to stop — no
    /// question of their own in the conversation's recent rows, or one that
    /// already has its reply or its stop.
    pub stopped: bool,
}

/// Stop the turn answering the caller's latest question.
///
/// Writes the stopped notice only for a question the caller wrote that has
/// neither a reply nor a stop; in every other case nothing is written and
/// `stopped` is `false`, so pressing stop twice — or after the reply landed —
/// is harmless. A stranger gets the same 404 every chat route gives.
///
/// Everything slow is resolved before the question is read, so the read and
/// the write are back to back: the reply landing between the two is the one
/// way a notice can be written beside a reply, and the turn closes its side
/// of that window by looking for the notice immediately before it stores.
///
/// # Errors
///
/// Returns `ResourceNotFound` when the conversation is not the caller's, and
/// database errors when the conversation cannot be read or the notice cannot
/// be written.
pub async fn stop_turn(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Json<StopTurnResponse>, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();
    let repos = &resources.common.repos;

    repos
        .chat
        .get_conversation(&conversation_id, &user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    let locale = resolve_user_locale(repos.users.as_ref(), auth.user_id).await;
    let notice = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_TURN_STOPPED, &locale);

    let rows = recent_rows(repos.chat.as_ref(), &conversation_id, &user_id, tenant_id).await?;
    let Some(question) = latest_question_by(&rows, &user_id) else {
        return Ok(Json(StopTurnResponse { stopped: false }));
    };
    if !matches!(
        question_state(&rows, &question.id),
        Some(QuestionState::Unanswered)
    ) {
        return Ok(Json(StopTurnResponse { stopped: false }));
    }

    let closes = stop_marker(&question.id)
        .ok_or_else(|| AppError::internal("the stopped notice could not name its question"))?;
    let params = AddMessageParams {
        tenant_id,
        conversation_id: &conversation_id,
        user_id: &user_id,
        role: "assistant",
        content: &notice,
        token_count: None,
        finish_reason: Some(STOPPED_TURN_FINISH_REASON),
        prompt_tokens: None,
        model: None,
        content_blocks: Some(&closes),
    };
    persist_assistant_response(
        repos.chat.as_ref(),
        repos.groups.as_ref(),
        &params,
        tenant_id,
    )
    .await?;

    info!(
        conversation_id = conversation_id.as_str(),
        question_id = question.id.as_str(),
        "the athlete stopped a chat turn"
    );
    Ok(Json(StopTurnResponse { stopped: true }))
}
