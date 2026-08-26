// ABOUTME: Web-admin HTTP handlers for system-wide settings — the auto-approval switch
// ABOUTME: Split from lib.rs; pairs with pierre_services::admin_settings, which holds the logic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The cookie-authenticated admin surface for the global auto-approval switch.
//!
//! Every handler authenticates through `WebAdminRoutes::authenticate_admin`
//! first — these change behaviour for every user of the deployment, so a valid
//! session is not sufficient on its own.
//!
//! Auto-approval is environment-shadowed: when `AUTO_APPROVE_USERS` is set in
//! the process environment it wins, and the GET response says so, letting the
//! UI present the value as fixed rather than accepting a write the next read
//! would discard.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::info;

use pierre_core::errors::AppError;
use pierre_services::admin_settings::{get_auto_approval_settings, set_auto_approval};

use super::WebAdminContext;

/// The settings surface's cookie-auth route table.
///
/// Owned here rather than in `WebAdminRoutes::routes` so a handler and the
/// path that reaches it stay in one file.
pub fn routes() -> Router<WebAdminContext> {
    Router::new().route(
        "/api/admin/settings/auto-approval",
        get(handle_get_auto_approval).put(handle_set_auto_approval),
    )
}

/// Handle getting auto-approval setting
pub async fn handle_get_auto_approval(
    headers: HeaderMap,
    State(resources): State<WebAdminContext>,
) -> Result<impl IntoResponse, AppError> {
    super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    let settings =
        get_auto_approval_settings(&resources.data, &resources.config.app_behavior).await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Auto-approval setting retrieved",
            "data": {
                "enabled": settings.enabled,
                "auto_approve_domains": settings.auto_approve_domains,
                // The console disables its toggle on this flag. Without it the
                // operator gets a control that accepts a change, reports success,
                // and silently reverts on the next read.
                "overridden_by_env": settings.overridden_by_env,
                "description": "When enabled, all new registrations are auto-approved. \
                    When disabled, only emails from auto_approve_domains are auto-approved."
            }
        })),
    )
        .into_response())
}

/// Handle setting auto-approval
pub async fn handle_set_auto_approval(
    headers: HeaderMap,
    State(resources): State<WebAdminContext>,
    Json(request): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let auth = super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    let enabled = request
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| AppError::invalid_input("Missing or invalid 'enabled' field"))?;

    info!(
        user_id = %auth.user_id,
        enabled = enabled,
        "Setting auto-approval"
    );

    set_auto_approval(&resources.data, enabled).await?;

    // Read the setting back rather than echoing the request. `AUTO_APPROVE_USERS`
    // in the environment takes precedence over the stored row by design, so the
    // write can persist while changing nothing: the old response reported the
    // requested value as fact, the UI showed success, and the toggle snapped back
    // on the next read. Reporting the effective value makes the override visible
    // instead of pretending the write took.
    let effective =
        get_auto_approval_settings(&resources.data, &resources.config.app_behavior).await?;

    info!(
        user_id = %auth.user_id,
        requested = enabled,
        effective = effective.enabled,
        overridden_by_env = effective.overridden_by_env,
        "Auto-approval setting updated"
    );

    let message = if effective.overridden_by_env {
        "Auto-approval is controlled by the AUTO_APPROVE_USERS environment variable; \
         the stored setting was saved but has no effect while that override is set."
            .to_owned()
    } else {
        format!(
            "Auto-approval has been {}",
            if effective.enabled {
                "enabled"
            } else {
                "disabled"
            }
        )
    };

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": message,
            "data": {
                "enabled": effective.enabled,
                "overridden_by_env": effective.overridden_by_env,
                "description": "When enabled, new user registrations are automatically approved without admin intervention"
            }
        })),
    )
        .into_response())
}
