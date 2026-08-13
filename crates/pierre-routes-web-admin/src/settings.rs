// ABOUTME: Web-admin HTTP handlers for system-wide settings — auto-approval and social insights
// ABOUTME: Split from lib.rs; pairs with pierre_services::admin_settings, which holds the logic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The cookie-authenticated admin surface for the two global switches.
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
use axum::Json;
use tracing::info;

use pierre_core::config::social::SocialInsightsConfig;
use pierre_core::errors::AppError;
use pierre_services::admin_settings::{
    get_auto_approval_settings, get_social_insights_config, reset_social_insights_config,
    set_auto_approval, set_social_insights_config,
};

use super::WebAdminContext;

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

// =========================================================================
// Social Insights Configuration Routes (web admin versions with cookie auth)
// =========================================================================

/// GET `/api/admin/settings/social-insights` - Get social insights configuration
pub async fn handle_get_social_insights_config(
    headers: HeaderMap,
    State(resources): State<WebAdminContext>,
) -> Result<impl IntoResponse, AppError> {
    super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    let config = get_social_insights_config(&resources.data).await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Social insights configuration retrieved",
            "data": config
        })),
    )
        .into_response())
}

/// PUT `/api/admin/settings/social-insights` - Update social insights configuration
pub async fn handle_set_social_insights_config(
    headers: HeaderMap,
    State(resources): State<WebAdminContext>,
    Json(config): Json<SocialInsightsConfig>,
) -> Result<impl IntoResponse, AppError> {
    let auth = super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    info!(
        user_id = %auth.user_id,
        "Updating social insights configuration"
    );

    set_social_insights_config(&resources.data, &config).await?;

    info!(
        user_id = %auth.user_id,
        "Social insights configuration updated"
    );

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Social insights configuration updated",
            "data": config
        })),
    )
        .into_response())
}

/// DELETE `/api/admin/settings/social-insights` - Reset social insights to defaults
pub async fn handle_reset_social_insights_config(
    headers: HeaderMap,
    State(resources): State<WebAdminContext>,
) -> Result<impl IntoResponse, AppError> {
    let auth = super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    info!(
        user_id = %auth.user_id,
        "Resetting social insights configuration to defaults"
    );

    let default_config = reset_social_insights_config(&resources.data).await?;

    info!(
        user_id = %auth.user_id,
        "Social insights configuration reset to defaults"
    );

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Social insights configuration reset to defaults",
            "data": default_config
        })),
    )
        .into_response())
}
