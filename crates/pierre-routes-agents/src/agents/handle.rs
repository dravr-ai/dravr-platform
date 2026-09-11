// ABOUTME: GET /api/agents/by-handle/{handle} — resolve an installed agent by its catalogue handle
// ABOUTME: The @handle route that later chat surfaces use to invite an agent into a conversation
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
use pierre_core::errors::AppError;
use pierre_core::models::agents::AgentHandle;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{AgentsCtx, MiddlewareCtx};

use super::types::AgentResponse;

/// Handle GET /api/agents/by-handle/:handle - Resolve an installed agent by
/// its catalogue handle (the `@handle` a user types to invite it).
///
/// Only an agent on the caller's own list resolves; a handle that exists in
/// the catalogue but was never installed answers 404, the same as an unknown
/// one, so the route does not leak the catalogue.
pub(super) async fn handle_get_by_handle<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(handle): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;
    let handle = AgentHandle::parse(&handle)?;

    let manager = super::get_agents_manager(&ctx);
    let agent = manager
        .find_installed_by_handle(&handle, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Installed coach @{handle}")))?;

    let agent_id = agent.id.to_string();
    let mut response: AgentResponse = agent.into();
    let (is_favorite, use_count, last_used_at) = manager
        .get_user_preferences(&agent_id, auth.user_id)
        .await?;
    response.is_favorite = is_favorite;
    response.use_count = use_count;
    response.last_used_at = last_used_at.map(|dt| dt.to_rfc3339());

    Ok((StatusCode::OK, Json(response)).into_response())
}
