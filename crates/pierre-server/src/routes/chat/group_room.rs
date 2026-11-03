// ABOUTME: Handler for POST /api/chat/groups/{group_id}/room — the caller's conversation in a group's room, opened when they hold none
// ABOUTME: Finds the caller's room row first, so Coach Home's "Write in the group" never pages the list nor makes a second room

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A group's human coach reaches the group's room from Coach Home. A coach
//! who never posted in a room they run holds no conversation in it, and the
//! one they hold is not necessarily on the first page of their list. This
//! route answers with the conversation the caller holds in the group — the
//! most recently active one — and creates it, the way `POST
//! /api/chat/conversations` creates a room, only when there is none. Asking
//! twice returns the same room, and so does asking twice at once: the create
//! looks again under the owner slot lock before it writes.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;
use pierre_services::conversation_forge::RoomOrigin;
use tracing::info;

use crate::mcp::resources::ServerContext;

use super::common::{conversation_response, get_tenant_id, verify_group_membership};
use super::conversations::{create_for, RoomSlot};
use super::dto::CreateConversationRequest;

/// Open the caller's conversation in `group_id`'s room: 200 with the one
/// they hold, else 201 with the one opened now. 403 when the caller is
/// neither a member nor the group's human coach.
pub(super) async fn open_group_room(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
    Path(group_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let group_id = group_id.trim();
    verify_group_membership(&resources, group_id, auth.user_id, tenant_id).await?;

    let caller = auth.user_id.to_string();
    if let Some(room) = resources
        .common
        .repos
        .chat
        .find_group_conversation(&caller, tenant_id, group_id)
        .await?
    {
        let response = conversation_response(&resources, room).await;
        return Ok((StatusCode::OK, Json(response)).into_response());
    }

    // Nothing held at the first look: open it, looking once more under the
    // owner slot lock, so a second device's open racing this one returns the
    // room the first one made instead of making another.
    let request = CreateConversationRequest {
        title: None,
        model: None,
        agent_id: None,
        group_id: Some(group_id.to_owned()),
        athlete_speaks_first: false,
    };
    let (response, origin) = create_for(
        &resources,
        &auth,
        tenant_id,
        &headers,
        &request,
        RoomSlot::CallersRoom,
    )
    .await?;
    if origin == RoomOrigin::Held {
        return Ok((StatusCode::OK, Json(response)).into_response());
    }
    info!(
        user_id = %auth.user_id,
        group_id,
        conversation_id = %response.id,
        "opened the caller's conversation in a group room"
    );
    Ok((StatusCode::CREATED, Json(response)).into_response())
}
