// ABOUTME: HTTP boundary for per-user rate-limit overrides (PUT/DELETE)
// ABOUTME: Thin HTTP boundary delegating writes to pierre_services::admin_ops
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-user rate-limit override admin endpoints.
//!
//! - `PUT /api/admin/users/{user_id}/rate-limit-override` — set or update
//!   a custom monthly cap (or null for no monthly ceiling), plus an audit
//!   note. Authentication enforces it on the user's JWT, cookie and channel
//!   requests in place of the tier's monthly limit.
//! - `DELETE /api/admin/users/{user_id}/rate-limit-override` — revert the
//!   user to their tier default.
//!
//! Both routes are mounted behind the cookie-admin middleware in
//! [`crate::AdminRoutes::cookie_admin_routes`]. The write logic lives in
//! [`pierre_services::admin_ops`] (`{set,clear}_user_rate_limit_override`)
//! so this HTTP surface and `pierre-cli user set-rate-limit/clear-rate-limit`
//! share one implementation.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, ErrorCode};
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

use crate::context::AdminApiContext;
use pierre_services::admin_ops;

/// Request body for `PUT /api/admin/users/{user_id}/rate-limit-override`.
///
/// `monthly_limit` accepts a positive integer (custom cap) or null (no
/// monthly ceiling). Zero is rejected — use null for unlimited. An unknown
/// or missing field is refused, so a request cannot set a cap other than
/// the one it names: a body without `monthly_limit` would otherwise read as
/// null and lift the user's ceiling.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRateLimitOverrideRequest {
    /// Custom monthly request cap. Null = unlimited monthly. The key is
    /// required even when its value is null.
    #[serde(deserialize_with = "Option::deserialize")]
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
    admin_ops::set_user_rate_limit_override(
        &context.repos,
        user_uuid,
        body.monthly_limit,
        body.note,
        Some(admin_user_id),
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
    let removed = admin_ops::clear_user_rate_limit_override(&context.repos, user_uuid).await?;
    info!(
        admin_id = %admin_user_id,
        target_user_id = %user_uuid,
        removed,
        "Per-user rate-limit override cleared via web admin"
    );
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "removed": removed})),
    )
        .into_response())
}
