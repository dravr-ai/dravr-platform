// ABOUTME: POST/DELETE handlers for per-message thumbs up/down chat feedback
// ABOUTME: Persists the caller's rating + optional reason, tenant + ownership gated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_core::models::UpsertMessageFeedbackParams;
use pierre_middleware::AuthenticatedUser;

use super::common::get_tenant_id;
use super::dto::{MessageFeedbackEntry, UpsertFeedbackRequest};

/// Set (or update) the caller's thumbs up/down feedback on a message.
///
/// Upsert keyed on `(message, user)`: re-rating overwrites the prior value and
/// refreshes the optional reason. Ownership is enforced in the repository — the
/// write only lands when the message belongs to a conversation the caller owns
/// in this tenant, otherwise a 404 is returned.
pub async fn upsert_feedback(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, message_id)): Path<(String, String)>,
    Json(request): Json<UpsertFeedbackRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    // Treat a blank/whitespace comment as "no reason given".
    let comment = request
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());

    let record = resources
        .common
        .repos
        .chat
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: &user_id,
            rating: request.rating.as_str(),
            comment,
        })
        .await?;

    let entry = MessageFeedbackEntry {
        message_id: record.message_id,
        rating: record.rating,
        comment: record.comment,
    };

    Ok((StatusCode::OK, Json(entry)).into_response())
}

/// Remove the caller's feedback on a message (thumbs toggle-off).
///
/// Idempotent: returns 204 even when there was no feedback to remove, so a
/// client optimistically clearing a rating never sees a spurious 404.
pub async fn delete_feedback(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    // Verify the caller owns the conversation, mirroring the messages-list path.
    resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    resources
        .common
        .repos
        .chat
        .delete_message_feedback(&message_id, &user_id, tenant_id)
        .await?;

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}
