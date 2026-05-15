// ABOUTME: Thin route layer for per-user rate-limit override (set + clear)
// ABOUTME: Logic lives in services::admin_ops; this file just shapes the HTTP boundary
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_auth::auth::AuthResult;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::AppError,
    mcp::resources::ServerContext,
    middleware::{extract_auth_from_headers, require_admin},
    services::admin_ops,
};

/// Request body for `PUT /api/admin/users/{user_id}/rate-limit-override`.
///
/// `daily_limit` and `monthly_limit` accept a positive integer (custom cap) or
/// null (unlimited for that dimension). Zero is rejected — use null for
/// unlimited.
#[derive(Debug, Deserialize)]
pub struct SetRateLimitOverrideRequest {
    /// Custom daily request cap. Null = unlimited daily.
    pub daily_limit: Option<u32>,
    /// Custom monthly request cap. Null = unlimited monthly.
    pub monthly_limit: Option<u32>,
    /// Operator-facing note explaining why the override exists.
    pub note: Option<String>,
}

async fn authenticate_admin(
    headers: &HeaderMap,
    resources: &Arc<ServerContext>,
) -> Result<AuthResult, AppError> {
    let auth = extract_auth_from_headers(headers, resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;
    Ok(auth)
}

/// PUT /api/admin/users/{user_id}/rate-limit-override — set or update.
///
/// # Errors
///
/// Returns `AppError` when the `user_id` path param is not a UUID, the admin
/// auth fails, or the underlying repository upsert errors.
pub async fn handle_set(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<SetRateLimitOverrideRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate_admin(&headers, &resources).await?;
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
    admin_ops::set_user_rate_limit_override(
        &resources.data(),
        auth.user_id,
        user_uuid,
        body.daily_limit,
        body.monthly_limit,
        body.note,
    )
    .await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "message": "Rate limit override applied"})),
    )
        .into_response())
}

/// DELETE /api/admin/users/{user_id}/rate-limit-override — revert to tier default.
///
/// # Errors
///
/// Returns `AppError` when the `user_id` path param is not a UUID, the admin
/// auth fails, or the underlying repository delete errors.
pub async fn handle_clear(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = authenticate_admin(&headers, &resources).await?;
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
    let removed =
        admin_ops::clear_user_rate_limit_override(&resources.data(), auth.user_id, user_uuid)
            .await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "removed": removed})),
    )
        .into_response())
}
