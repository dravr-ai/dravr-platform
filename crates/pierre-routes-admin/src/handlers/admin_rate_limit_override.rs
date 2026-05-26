// ABOUTME: HTTP boundary for per-user rate-limit overrides (PUT/DELETE)
// ABOUTME: Inlined service layer operates directly on RepositoryRegistry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-user rate-limit override admin endpoints.
//!
//! - `PUT /api/admin/users/{user_id}/rate-limit-override` — set or update
//!   a custom daily / monthly cap (or null for "unlimited at that
//!   dimension"), plus an audit note.
//! - `DELETE /api/admin/users/{user_id}/rate-limit-override` — revert the
//!   user to their tier default.
//!
//! Both routes are mounted behind the cookie-admin middleware in
//! [`crate::AdminRoutes::cookie_admin_routes`]. The service layer that
//! was `crate::services::admin_ops::{set,clear}_user_rate_limit_override`
//! in `pierre-server` is inlined here (~50 LOC) because it only touches
//! `repos.users` and `repos.user_rate_limit_overrides`.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_database::repositories::UserRateLimitOverride;
use pierre_database::{AuthRepos, UsageRepos};
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

use crate::context::AdminApiContext;

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

/// Extract the admin user UUID from a cookie-auth-synthesized
/// [`ValidatedAdminToken`]. The cookie-admin middleware encodes the
/// originating user as `token_id = "cookie:<uuid>"`.
fn admin_user_id_from_token(admin_token: &ValidatedAdminToken) -> Result<Uuid, AppError> {
    let raw = admin_token
        .token_id
        .strip_prefix("cookie:")
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "Rate-limit override endpoints require cookie authentication",
            )
        })?;
    Uuid::parse_str(raw).map_err(|e| {
        AppError::internal(format!(
            "Cookie admin token carried a non-UUID user id: {e}"
        ))
    })
}

/// `PUT /api/admin/users/{user_id}/rate-limit-override` — set or update.
///
/// # Errors
///
/// Returns `AppError` when the `user_id` path param is not a UUID, the admin
/// auth fails, or the underlying repository upsert errors.
pub async fn handle_set(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(body): Json<SetRateLimitOverrideRequest>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ManageUsers)?;
    let admin_user_id = admin_user_id_from_token(&admin_token)?;
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
    let auth = context.repos.auth_repos();
    let usage = context.repos.usage_repos();
    set_user_rate_limit_override(
        &auth,
        &usage,
        admin_user_id,
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

/// `DELETE /api/admin/users/{user_id}/rate-limit-override` — revert to tier default.
///
/// # Errors
///
/// Returns `AppError` when the `user_id` path param is not a UUID, the admin
/// auth fails, or the underlying repository delete errors.
pub async fn handle_clear(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ManageUsers)?;
    let admin_user_id = admin_user_id_from_token(&admin_token)?;
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
    let usage = context.repos.usage_repos();
    let removed = clear_user_rate_limit_override(&usage, admin_user_id, user_uuid).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "removed": removed})),
    )
        .into_response())
}

// =========================================================================
// Inlined service layer — was in crate::services::admin_ops in pierre-server
// =========================================================================

/// Set or update a per-user rate-limit override.
///
/// Validates that limit values are positive or null; rejects zero. Verifies
/// the target user exists before persisting. Audit-logs the admin actor.
async fn set_user_rate_limit_override(
    auth: &AuthRepos,
    usage: &UsageRepos,
    admin_user_id: Uuid,
    target_user_id: Uuid,
    daily_limit: Option<u32>,
    monthly_limit: Option<u32>,
    note: Option<String>,
) -> Result<(), AppError> {
    if matches!(daily_limit, Some(0)) || matches!(monthly_limit, Some(0)) {
        return Err(AppError::invalid_input(
            "Rate limit values must be positive integers; use null for unlimited",
        ));
    }

    // Verify the target user exists (admin views are global).
    auth.users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    let now = Utc::now();
    let row = UserRateLimitOverride {
        user_id: target_user_id,
        daily_limit,
        monthly_limit,
        note,
        set_by: Some(admin_user_id),
        set_at: now,
        updated_at: now,
    };

    usage
        .user_rate_limit_overrides
        .upsert(&row)
        .await
        .map_err(|e| AppError::internal(format!("Failed to persist rate-limit override: {e}")))?;

    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        daily_limit = ?daily_limit,
        monthly_limit = ?monthly_limit,
        "Per-user rate-limit override set"
    );

    Ok(())
}

/// Remove a per-user rate-limit override; user reverts to the tier default.
/// Returns `true` when an override row existed and was removed.
async fn clear_user_rate_limit_override(
    usage: &UsageRepos,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<bool, AppError> {
    let removed = usage
        .user_rate_limit_overrides
        .delete(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear rate-limit override: {e}")))?;

    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        removed,
        "Per-user rate-limit override cleared"
    );

    Ok(removed)
}
