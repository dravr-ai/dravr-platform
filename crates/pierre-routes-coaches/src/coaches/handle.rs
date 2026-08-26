// ABOUTME: GET /api/coaches/by-handle/{handle} — resolve an installed coach by its catalogue handle
// ABOUTME: The @handle route that later chat surfaces use to invite a coach into a conversation
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
use pierre_core::models::coaches::CoachHandle;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{CoachesCtx, MiddlewareCtx};

use super::types::CoachResponse;

/// Handle GET /api/coaches/by-handle/:handle - Resolve an installed coach by
/// its catalogue handle (the `@handle` a user types to invite it).
///
/// Only a coach on the caller's own list resolves; a handle that exists in
/// the catalogue but was never installed answers 404, the same as an unknown
/// one, so the route does not leak the catalogue.
pub(super) async fn handle_get_by_handle<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(handle): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;
    let handle = CoachHandle::parse(&handle)?;

    let manager = super::get_coaches_manager(&ctx);
    let coach = manager
        .find_installed_by_handle(&handle, auth.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Installed coach @{handle}")))?;

    let coach_id = coach.id.to_string();
    let mut response: CoachResponse = coach.into();
    let (is_favorite, use_count, last_used_at) = manager
        .get_user_preferences(&coach_id, auth.user_id)
        .await?;
    response.is_favorite = is_favorite;
    response.use_count = use_count;
    response.last_used_at = last_used_at.map(|dt| dt.to_rfc3339());

    Ok((StatusCode::OK, Json(response)).into_response())
}
