// ABOUTME: POST/DELETE handlers for /api/chat/conversations/{id}/read — the caller's read marker
// ABOUTME: POST advances it (never backwards); DELETE clears it, which is "mark unread"
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
use serde::Deserialize;

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;

use super::common::get_tenant_id;

/// Body of `POST …/read`. Optional: an empty body marks the whole thread read.
#[derive(Debug, Default, Deserialize)]
pub struct MarkReadRequest {
    /// The newest message the caller has seen. Absent means the newest
    /// `user`/`assistant` row of the conversation.
    #[serde(default)]
    pub up_to_message_id: Option<String>,
}

/// Advance the caller's read marker.
///
/// Monotonic: a client re-marking an older row than the marker already
/// covers changes nothing, so two tabs racing cannot resurrect unread rows.
/// A stranger, and a message id that is not in this conversation, both get
/// 404 — the existence of a thread is never disclosed to someone outside it.
pub async fn mark_read(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    body: Option<Json<MarkReadRequest>>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let request = body.map(|Json(request)| request).unwrap_or_default();
    let up_to = request
        .up_to_message_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());

    let marked = resources
        .common
        .repos
        .chat
        .mark_conversation_read(
            &conversation_id,
            &auth.user_id.to_string(),
            tenant_id,
            up_to,
        )
        .await?;
    if !marked {
        return Err(AppError::not_found("Conversation not found"));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// Clear the caller's read marker — mark the thread unread.
///
/// Every `user`/`assistant` row counts as unread again until the thread is
/// opened. Idempotent for a participant; a stranger gets 404.
pub async fn mark_unread(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    let cleared = resources
        .common
        .repos
        .chat
        .clear_conversation_read_marker(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?;
    if !cleared {
        return Err(AppError::not_found("Conversation not found"));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}
